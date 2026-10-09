//! Отладка: прогнать сцену в модели и напечатать дамп каждого тика (формат эталона).
//!   cargo run --release --example model_dump -- <scene.json> [тиков]
use std::path::PathBuf;

use factorio_distributed_core::fidelity;
use factorio_distributed_core::world::World;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let scene = fidelity::load_scene(&PathBuf::from(&a[0]))?;
    let ticks: u64 = a.get(1).map(|s| s.parse()).transpose()?.unwrap_or(scene.ticks);
    let mut w = World::from_scene(&scene, factorio_distributed_core::world::proto::Prototypes::bundled())?;
    println!("{}", serde_json::to_string(&w.dump())?);
    for _ in 0..ticks {
        w.step();
        println!("{}", serde_json::to_string(&w.dump())?);
    }
    Ok(())
}
