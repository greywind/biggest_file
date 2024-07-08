use std::io::{stdout, Stdout, Write};
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

enum CounterMessage {
    DirectoryAdded,
    DirectoryFinished,
}

enum FileSizeMessage {
    FileSize(FileSizeStruct),
    Finished,
}

enum OutputMessage {
    Counter(usize, usize),
    BiggestFile(FileSizeStruct),
    Finished,
}

pub async fn find_the_biggest_file(paths: &[String]) -> Result<FileSizeStruct, Box<dyn std::error::Error>> {
    let searcher = BiggestFileSearcher::new();
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
    counter_tx: mpsc::Sender<CounterMessage>,
    output_tx: mpsc::Sender<OutputMessage>,
    biggest_file: Arc<Mutex<Option<FileSizeStruct>>>,
}

impl BiggestFileSearcher {
    const MAX_DOP: usize = 10;
    fn new() -> Self {
        let (paths_tx, paths_rx) = async_channel::unbounded();
        let (file_size_tx, file_size_rx) = mpsc::channel(BiggestFileSearcher::MAX_DOP);
        let (counter_tx, counter_rx) = mpsc::channel(BiggestFileSearcher::MAX_DOP);
        let (output_tx, output_rx) = mpsc::channel(BiggestFileSearcher::MAX_DOP + 2);
        let result = Self {
            paths_tx,
            paths_rx,
            file_size_tx,
            counter_tx,
            output_tx,
            biggest_file: Arc::new(Mutex::new(None)),
        };
        result.start_counter_task(counter_rx);
        result.start_receive_file_size_worker(file_size_rx);
        result.start_output_worker(output_rx);

        result
    }

    async fn add_path_to_search(path: PathBuf, paths_tx: async_channel::Sender<PathBuf>, counter_tx: mpsc::Sender<CounterMessage>) {
        counter_tx.send(CounterMessage::DirectoryAdded).await.expect("Failed to send counter");
        paths_tx.send(path).await.expect("Failed to send path");
    }

    pub async fn biggest_file(&self) -> Option<FileSizeStruct> {
        let biggest_file = self.biggest_file.lock().await;
        biggest_file.clone()
    }

    async fn find_the_biggest_file(&self, paths: &[String]) -> () {
        for path in paths {
            Self::add_path_to_search(PathBuf::from(path), self.paths_tx.clone(), self.counter_tx.clone()).await;
        }

        let mut tasks = Vec::with_capacity(BiggestFileSearcher::MAX_DOP);
        for _ in 0..BiggestFileSearcher::MAX_DOP {
            tasks.push(self.start_file_searcher_worker());
        }

        for task in tasks {
            task.await.expect("Failed to join task");
        }
    }

    fn start_counter_task(&self, mut counter_rx: mpsc::Receiver<CounterMessage>) {
        let paths_tx = self.paths_tx.clone();
        let file_size_tx = self.file_size_tx.clone();
        let output_tx = self.output_tx.clone();
        tokio::spawn(async move {
            let mut total = 0;
            let mut finished = 0;
            while let Some(value) = counter_rx.recv().await {
                match value {
                    CounterMessage::DirectoryAdded => {
                        total += 1;
                    }
                    CounterMessage::DirectoryFinished => {
                        finished += 1;
                    }
                }
                output_tx.send(OutputMessage::Counter(finished, total)).await.expect("Failed to send counter to output");
                if total == finished {
                    paths_tx.close();
                    file_size_tx.send(Finished).await.expect("Failed to send Finished");
                    file_size_tx.closed().await;
                    output_tx.send(OutputMessage::Finished).await.expect("Failed to send Finished to output");
                    output_tx.closed().await;
                    break;
                };
            };
        });
    }

    fn start_file_searcher_worker(&self) -> tokio::task::JoinHandle<()> {
        let counter_tx = self.counter_tx.clone();
        let paths_rx = self.paths_rx.clone();
        let paths_tx = self.paths_tx.clone();
        let file_size_tx = self.file_size_tx.clone();

        let join_handle = tokio::spawn(async move {
            while let Ok(path) = paths_rx.recv().await {
                let mut read_dir = tokio::fs::read_dir(path.deref()).await.unwrap();
                while let Some(entry) = read_dir.next_entry().await.unwrap() {
                    let file_type = entry.file_type().await.unwrap();
                    if file_type.is_dir() {
                        Self::add_path_to_search(entry.path(), paths_tx.clone(), counter_tx.clone()).await;
                        continue;
                    }

                    let metadata = entry.metadata().await;
                    if let Err(_) = metadata {
                        continue;
                    }

                    let file_size = FileSizeStruct {
                        size: metadata.unwrap().len(),
                        path: entry.path().to_str().unwrap().to_string(),
                    };
                    file_size_tx.send(FileSize(file_size)).await.expect("Failed to send file size");
                }
                counter_tx.send(CounterMessage::DirectoryFinished).await.expect("Failed to send counter");
            }
        });

        join_handle
    }

    fn start_receive_file_size_worker(&self, mut file_size_rx: mpsc::Receiver<FileSizeMessage>) {
        let biggest_file_arc = self.biggest_file.clone();
        let output_tx = self.output_tx.clone();
        tokio::spawn(async move {
            while let Some(message) = file_size_rx.recv().await {
                match message {
                    FileSize(file_size) => {
                        let mut biggest_file_mutex = biggest_file_arc.lock().await;
                        if let Some(biggest_file) = &*biggest_file_mutex {
                            if file_size.size <= biggest_file.size {
                                continue;
                            }
                        }

                        let file_size_clone = file_size.clone();
                        *biggest_file_mutex = Some(file_size);
                        output_tx.send(OutputMessage::BiggestFile(file_size_clone)).await.expect("Failed to send biggest file to output");
                    }
                    Finished => {
                        file_size_rx.close();
                        break;
                    }
                }
            }
        });
    }

    fn start_output_worker(&self, mut output_rx: mpsc::Receiver<OutputMessage>) {
        tokio::spawn(async move {
            struct State {
                finished: usize,
                total: usize,
                biggest_file: Option<FileSizeStruct>,
            }
            let mut state = State {
                finished: 0,
                total: 0,
                biggest_file: None,
            };

            let mut stdout = stdout();

            fn clear_output(t: &mut Stdout, state: &State) {
                if state.total == 0 {
                    return;
                }

                let lines = if state.biggest_file.is_none() { 1 } else { 2 };
                for _ in 0..lines {
                    t.write_all("\x1B[1A\x1B[2K".as_bytes()).unwrap();
                }
            }

            fn output(t: &mut Stdout, state: &State) {
                if state.total == 0 {
                    return;
                }

                t.write_fmt(format_args!("{}/{} ({}%)\n", state.finished, state.total, (state.finished as f32 / state.total as f32 * 100.0).floor())).unwrap();
                if let Some(biggest_file) = &state.biggest_file {
                    t.write_fmt(format_args!("Biggest file: '{}' with size: {} bytes\n", biggest_file.path, biggest_file.size)).unwrap();
                }
            }

            while let Some(message) = output_rx.recv().await {
                clear_output(&mut stdout, &state);
                match message {
                    OutputMessage::Counter(finished, total) => {
                        state.finished = finished;
                        state.total = total;
                    }
                    OutputMessage::BiggestFile(file_size) => {
                        state.biggest_file = Some(file_size);
                    }
                    OutputMessage::Finished => {
                        output_rx.close();
                        break;
                    }
                }
                output(&mut stdout, &state);
            }
        });
    }
}