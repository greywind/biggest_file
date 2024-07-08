mod biggest_file_searcher;

use std::fs;

#[tokio::main]
async fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() < 2 {
        eprintln!("Usage: {} <path> [...<path2>]", args[0]);
        std::process::exit(1);
    }

    let paths = &args[1..];

    for path in paths {
        if fs::metadata(path).is_err() {
            eprintln!("Path does not exist: {}", path);
            std::process::exit(1);
        }
    }

    println!("I'll find the biggest file in {}!", paths.join(", "));
    let result = biggest_file_searcher::find_the_biggest_file(paths).await.expect("Error finding the biggest file");
    println!("The biggest file is: '{}' with size: {} bytes", result.path, result.size);
}
