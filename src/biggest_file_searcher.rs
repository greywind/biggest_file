use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Arc};
use tokio::sync::{mpsc, RwLock, Mutex};

#[derive(Clone)]
pub struct FileSize {
    pub path: String,
    pub size: u64,
}

pub async fn find_the_biggest_file(path: String) -> Result<FileSize, Box<dyn std::error::Error>> {
    let mut searcher = BiggestFileSearcher::new();
    searcher.receive_file_size().await;
    searcher.find_the_biggest_file(path).await?;
    match searcher.biggest_file().await {
        Some(file_size) => Ok(file_size),
        None => Err("No files found".into()),
    }
}

struct BiggestFileSearcherImpl {
    paths_tx: async_channel::Sender<PathBuf>,
    paths_rx: async_channel::Receiver<PathBuf>,
    file_size_tx: mpsc::Sender<FileSize>,
    file_size_rx: Arc<Mutex<mpsc::Receiver<FileSize>>>,
    counter_tx: mpsc::Sender<isize>,
    biggest_file: Arc<Mutex<Option<FileSize>>>,
}

struct BiggestFileSearcher {
    arc: Arc<RwLock<BiggestFileSearcherImpl>>,
}

impl BiggestFileSearcherImpl {
    const BUFFER_SIZE: usize = 100;
    fn new() -> Self {
        let (paths_tx, paths_rx) = async_channel::unbounded();
        let (file_size_tx, file_size_rx) = mpsc::channel(BiggestFileSearcherImpl::BUFFER_SIZE);
        let (counter_tx, counter_rx) = mpsc::channel(BiggestFileSearcherImpl::BUFFER_SIZE);
        let result = Self {
            paths_tx,
            paths_rx,
            file_size_tx,
            file_size_rx: Arc::new(Mutex::new(file_size_rx)),
            counter_tx,
            biggest_file: Arc::new(Mutex::new(None)),
        };
        result.start_counter_task(counter_rx);

        result
    }

    async fn add_path_to_search(&mut self, path: PathBuf) {
        self.counter_tx.send(1).await.expect("Failed to send counter");
        self.paths_tx.send(path).await.expect("Failed to send path");
    }

    fn start_counter_task(&self, mut counter_rx: mpsc::Receiver<isize>) {
        let paths_tx = self.paths_tx.clone();
        let file_size_tx = self.file_size_tx.clone();
        tokio::spawn(async move {
            let mut counter = 0;
            while let Some(value) = counter_rx.recv().await {
                counter += value;
                println!("Counter: {}", counter);
                if counter == 0 {
                    paths_tx.close();
                    file_size_tx.closed().await;
                    break;
                }
            }
        });
    }
}

impl BiggestFileSearcher {
    pub fn new() -> Self {
        let searcher = BiggestFileSearcherImpl::new();
        let arc = Arc::new(RwLock::new(searcher));
        Self { arc }
    }

    pub async fn biggest_file(&self) -> Option<FileSize> {
        println!("biggest_file: Taking read lock");
        let searcher = self.arc.read().await;
        let biggest_file = searcher.biggest_file.lock().await;
        println!("biggest_file: Releasing read lock");
        biggest_file.clone()
    }

    async fn find_the_biggest_file(&mut self, path: String) -> Result<(), Box<dyn std::error::Error>> {
        println!("find_the_biggest_file: Taking read lock");
        let searcher = self.arc.read().await;
        let counter_tx = searcher.counter_tx.clone();
        let paths_rx = searcher.paths_rx.clone();
        let file_size_tx = searcher.file_size_tx.clone();
        drop(searcher);
        println!("find_the_biggest_file: Releasing read lock");

        let arc = self.arc.clone();

        let path = PathBuf::from(path);
        tokio::spawn(async move { arc.write().await.add_path_to_search(path).await; });

        let arc = self.arc.clone();
        tokio::spawn(async move {
            while let Ok(path) = paths_rx.recv().await {
                println!("find_the_biggest_file: Received path: {:?}", path.to_str().unwrap());
                let mut read_dir = tokio::fs::read_dir(path.deref()).await.unwrap();
                while let Some(entry) = read_dir.next_entry().await.unwrap() {
                    println!("find_the_biggest_file: Entry: {:?}", entry.path().to_str().unwrap());

                    let file_type = entry.file_type().await.unwrap();
                    if file_type.is_dir() {
                        println!("find_the_biggest_file: Adding path to search: {:?}", entry.path());
                        let mut searcher = arc.write().await;
                        searcher.add_path_to_search(entry.path()).await;
                        continue;
                    }

                    let file_size = FileSize {
                        size: entry.metadata().await.unwrap().len(),
                        path: entry.path().to_str().unwrap().to_string(),
                    };
                    file_size_tx.send(file_size).await.expect("Failed to send file size");

                }
                counter_tx.send(-1).await.expect("Failed to send counter");
            }
        }).await.expect("Failed to spawn task");


        Ok(())
    }

    async fn receive_file_size(&self) {
        println!("receive_file_size: Taking read lock");
        let arc = self.arc.read().await;
        let file_size_rx_arc = arc.file_size_rx.clone();
        let biggest_file_arc = arc.biggest_file.clone();
        println!("receive_file_size: Releasing read lock");
        tokio::spawn(async move {
            let mut file_size_rx = file_size_rx_arc.lock().await;
            loop {
                let file_size = match file_size_rx.recv().await {
                    Some(file_size) => file_size,
                    None => break,
                };

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
        });
    }
}