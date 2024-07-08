use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};

use crate::biggest_file_searcher::FileSizeMessage::{FileSize, Finished};

#[derive(Clone)]
pub struct FileSizeStruct {
    pub path: String,
    pub size: u64,
}

enum FileSizeMessage {
    FileSize(FileSizeStruct),
    Finished,
}

pub async fn find_the_biggest_file(paths: &[String]) -> Result<FileSizeStruct, Box<dyn std::error::Error>> {
    let mut searcher = BiggestFileSearcher::new();
    searcher.receive_file_size().await;
    searcher.find_the_biggest_file(paths).await;
    match searcher.biggest_file().await {
        Some(file_size) => Ok(file_size),
        None => Err("No files found".into()),
    }
}

struct BiggestFileSearcher {
    paths_tx: async_channel::Sender<PathBuf>,
    paths_rx: async_channel::Receiver<PathBuf>,
    file_size_tx: mpsc::Sender<FileSizeMessage>,
    file_size_rx: Arc<Mutex<mpsc::Receiver<FileSizeMessage>>>,
    counter_tx: mpsc::Sender<isize>,
    counter_rx: Arc<Mutex<mpsc::Receiver<isize>>>,
    biggest_file: Arc<Mutex<Option<FileSizeStruct>>>,
}

impl BiggestFileSearcher {
    const MAX_DOP: usize = 10;
    fn new() -> Self {
        let (paths_tx, paths_rx) = async_channel::unbounded();
        let (file_size_tx, file_size_rx) = mpsc::channel(BiggestFileSearcher::MAX_DOP);
        let (counter_tx, counter_rx) = mpsc::channel(BiggestFileSearcher::MAX_DOP);
        let result = Self {
            paths_tx,
            paths_rx,
            file_size_tx,
            file_size_rx: Arc::new(Mutex::new(file_size_rx)),
            counter_tx,
            counter_rx: Arc::new(Mutex::new(counter_rx)),
            biggest_file: Arc::new(Mutex::new(None)),
        };

        result
    }

    async fn add_path_to_search(path: PathBuf, paths_tx: async_channel::Sender<PathBuf>, counter_tx: mpsc::Sender<isize>) {
        counter_tx.send(1).await.expect("Failed to send counter");
        paths_tx.send(path).await.expect("Failed to send path");
    }

    pub async fn biggest_file(&self) -> Option<FileSizeStruct> {
        println!("biggest_file: Taking read lock");
        let biggest_file = self.biggest_file.lock().await;
        println!("biggest_file: Releasing read lock");
        biggest_file.clone()
    }

    async fn find_the_biggest_file(&mut self, paths: &[String]) -> () {
        for path in paths {
            Self::add_path_to_search(PathBuf::from(path), self.paths_tx.clone(), self.counter_tx.clone()).await;
        }

        for _ in 0..BiggestFileSearcher::MAX_DOP {
            self.start_file_searcher_worker().await;
        }

        self.start_counter_task().await;
    }

    async fn start_counter_task(&self) {
        let paths_tx = self.paths_tx.clone();
        let file_size_tx = self.file_size_tx.clone();
        let counter_rx_mutex = self.counter_rx.clone();

        let mut counter = 0;
        while let Some(value) = counter_rx_mutex.lock().await.recv().await {
            counter += value;
            println!("Counter: {}", counter);
            if counter == 0 {
                println!("Counter task finishing");
                paths_tx.close();
                println!("Counter task waiting for file_size_tx to close");
                file_size_tx.send(Finished).await.expect("Failed to send Finished");
                file_size_tx.closed().await;
                println!("Counter task breaking");
                break;
            };
        };
        println!("Counter task finished");
    }

    async fn start_file_searcher_worker(&mut self) {
        println!("find_the_biggest_file: Taking read lock");
        let counter_tx = self.counter_tx.clone();
        let paths_rx = self.paths_rx.clone();
        let paths_tx = self.paths_tx.clone();
        let file_size_tx = self.file_size_tx.clone();
        println!("find_the_biggest_file: Releasing read lock");

        tokio::spawn(async move {
            while let Ok(path) = paths_rx.recv().await {
                println!("find_the_biggest_file: Received path: {:?}", path.to_str().unwrap());
                let mut read_dir = tokio::fs::read_dir(path.deref()).await.unwrap();
                while let Some(entry) = read_dir.next_entry().await.unwrap() {
                    println!("find_the_biggest_file: Entry: {:?}", entry.path().to_str().unwrap());

                    let file_type = entry.file_type().await.unwrap();
                    if file_type.is_dir() {
                        println!("find_the_biggest_file: Adding path to search: {:?}", entry.path());
                        Self::add_path_to_search(entry.path(), paths_tx.clone(), counter_tx.clone()).await;
                        continue;
                    }

                    let file_size = FileSizeStruct {
                        size: entry.metadata().await.unwrap().len(),
                        path: entry.path().to_str().unwrap().to_string(),
                    };
                    file_size_tx.send(FileSize(file_size)).await.expect("Failed to send file size");
                }
                counter_tx.send(-1).await.expect("Failed to send counter");
            }
        });
    }

    async fn receive_file_size(&self) {
        println!("receive_file_size: Taking read lock");
        let file_size_rx_arc = self.file_size_rx.clone();
        let biggest_file_arc = self.biggest_file.clone();
        println!("receive_file_size: Releasing read lock");
        tokio::spawn(async move {
            let mut file_size_rx = file_size_rx_arc.lock().await;
            while let Some(message) = file_size_rx.recv().await {
                 match message {
                    FileSize(file_size) => {
                        println!("receive_file_size: File size: {:?}, File path: {:?}", file_size.size, file_size.path);
                        let mut biggest_file_mutex = biggest_file_arc.lock().await;
                        if let Some(biggest_file) = &*biggest_file_mutex {
                            if file_size.size > biggest_file.size {
                                *biggest_file_mutex = Some(file_size);
                            }
                        } else {
                            *biggest_file_mutex = Some(file_size);
                        }
                    }
                    Finished => {
                        println!("receive_file_size: Finished");
                        file_size_rx.close();
                        break;
                    }
                }
            }
        });
    }
}