use std::collections::VecDeque;
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
    paths_to_check: VecDeque<PathBuf>,
    biggest_file: Option<FileSize>,
}

impl BiggestFileSearcher {
    fn new(path: String) -> Self {
        Self {
            paths_to_check: VecDeque::from([PathBuf::from(path)]),
            biggest_file: None,
        }
    }

    async fn find_the_biggest_file(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        while let Some(path) = self.paths_to_check.pop_front() {
            let mut read_dir = tokio::fs::read_dir(path.deref()).await?;
            loop {
                let entry = read_dir.next_entry().await?;
                let entry = match entry {
                    Some(entry) => entry,
                    None => break,
                };

                let file_type = entry.file_type().await?;
                if file_type.is_dir() {
                    self.paths_to_check.push_back(entry.path());
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
        }

        Ok(())
    }

}