use std::fs;

#[tokio::main]
async fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 2 {
        eprintln!("Usage: {} <path>", args[0]);
        std::process::exit(1);
    }

    let path = &args[1];

    if fs::metadata(path).is_err() {
        eprintln!("Path does not exist: {}", path);
        std::process::exit(1);
    }

    println!("I'll find the biggest file in {}!", path);
}
