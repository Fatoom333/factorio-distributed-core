//! Сверка модели мира с эталоном из игры.
//!
//!   fidelity <scene.json> <dump.jsonl> [--all]
//!
//! С `--all` дополнительно печатает все тики с расхождениями (модель идёт дальше
//! своим ходом) — для отладки.
//! Печатает число совпавших тиков и первое расхождение (тик, сущность,
//! ожидалось/получилось). Код выхода 1, если есть расхождение.

use std::path::PathBuf;

use factorio_distributed_core::fidelity;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let all = args.iter().any(|a| a == "--all");
    let args: Vec<String> = args.into_iter().filter(|a| a != "--all").collect();
    if args.len() != 2 {
        eprintln!("использование: fidelity <scene.json> <dump.jsonl> [--all]");
        std::process::exit(2);
    }
    let scene = fidelity::load_scene(&PathBuf::from(&args[0]))?;
    let dump = fidelity::load_dump(&PathBuf::from(&args[1]))?;
    let r = fidelity::run(&scene, &dump)?;
    println!("{}: совпало {} из {} тиков", r.scene, r.matched, r.total_ticks);
    if all {
        let ms = fidelity::all_mismatches(&scene, &dump)?;
        println!("тиков с расхождением: {}", ms.len());
        for (t, d) in ms.iter().take(200) {
            println!("  {t}: {d}");
        }
    }
    match r.first_mismatch {
        None => Ok(()),
        Some(m) => {
            println!("первое расхождение — {m}");
            std::process::exit(1);
        }
    }
}
