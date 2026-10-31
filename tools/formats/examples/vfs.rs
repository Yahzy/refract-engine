//! Inspect the virtual filesystem.
//!
//! cargo run -p refract-formats --example vfs -- paths
//! cargo run -p refract-formats --example vfs -- find "maps/*.bsp"
//! cargo run -p refract-formats --example vfs -- read models/props/barrel01.mdl
//! cargo run -p refract-formats --example vfs -- cat materials/concrete/concretefloor001a.vmt

use std::path::Path;
use std::time::Instant;

use refract_formats::{Steam, mount_default};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();

    let start = Instant::now();
    let steam = Steam::locate();
    let fs = mount_default(Path::new("game"), steam.as_ref());
    println!("mounted in {:?}", start.elapsed());

    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["paths"] | [] => {
            for line in fs.search_paths() {
                println!("{line}");
            }
        }
        ["find", pattern, rest @ ..] => {
            let found = fs.find(pattern, rest.first().copied());
            for dir in &found.dirs {
                println!("{dir}/");
            }
            for file in &found.files {
                println!("{file}");
            }
            println!("-- {} dirs, {} files", found.dirs.len(), found.files.len());
        }
        ["read", path] => match fs.read(path, None) {
            Some(data) => println!("{path}: {} bytes, header {:02x?}", data.len(), &data[..data.len().min(8)]),
            None => println!("{path}: not found"),
        },
        ["extract", path, out] => match fs.read(path, None) {
            Some(data) => std::fs::write(out, data).expect("write"),
            None => println!("{path}: not found"),
        },
        ["cat", path] => match fs.read(path, None) {
            Some(data) => println!("{}", String::from_utf8_lossy(&data)),
            None => println!("{path}: not found"),
        },
        _ => eprintln!("usage: vfs [paths | find <pattern> [pathid] | read <path> | extract <path> <file> | cat <path>]"),
    }
}
