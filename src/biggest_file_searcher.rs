use std::ops::Deref;
use std::path::PathBuf;

pub struct FileSize {
    pub path: String,
    pub size: u64,
}

pub async fn find_the_biggest_file(path: String) -> Result<FileSize, Box<dyn std::error::Error>> {
    let mut searcher = BiggestFileSearcher::new(path);
    searcher.find_the_biggest_file().await?;
    match searcher.biggest_file {
        Some(file_size) => Ok(file_size),
        None => Err("No files found".into()),
    }
}

struct BiggestFileSearcher {
    paths_tx: async_channel::Sender<PathBuf>,
    paths_rx: async_channel::Receiver<PathBuf>,
    biggest_file: Option<FileSize>,
    counter: usize,
}

impl BiggestFileSearcher {
    const BUFFER_SIZE: usize = 100;
    fn new(path: String) -> Self {
        let (paths_tx, paths_rx) = async_channel::bounded(BiggestFileSearcher::BUFFER_SIZE);
        let mut result = Self {
            paths_tx,
            paths_rx,
            biggest_file: None,
            counter: 0,
        };
        let path = PathBuf::from(path);
        result.add_path_to_search(path);
        result
    }

    fn add_path_to_search(&mut self, path: PathBuf) {
        let paths_tx = self.paths_tx.clone();
        self.counter += 1;
        tokio::spawn( async move { paths_tx.send(path).await });
    }

    async fn find_the_biggest_file(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        while let Ok(path) = self.paths_rx.recv().await {
            self.counter -= 1;
            let mut read_dir = tokio::fs::read_dir(path.deref()).await?;
            loop {
                let entry = read_dir.next_entry().await?;
                let entry = match entry {
                    Some(entry) => entry,
                    None => break,
                };

                let file_type = entry.file_type().await?;
                if file_type.is_dir() {
                    self.add_path_to_search(entry.path());
                    continue;
                }

                let file_size = entry.metadata().await?.len();
                if self.biggest_file.is_none() || file_size > self.biggest_file.as_ref().unwrap().size {
                    self.biggest_file = Some(FileSize {
                        size: file_size,
                        path: entry.path().to_str().unwrap().to_string(),
                    });
                }
            }
            if self.counter == 0 {
                break;
            }
        }

        Ok(())
    }

}