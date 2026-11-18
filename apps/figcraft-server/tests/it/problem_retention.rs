use figcraft_server::config::Config;
use figcraft_server::store::{FILES, Store};

#[test]
fn startup_problem_retention() {
    let root = std::env::temp_dir().join(format!("figcraft-problem-cap-{}", std::process::id()));
    assert!(!root.exists());
    let result = (|| -> Result<(), String> {
        for count in [3u32, 200, 201] {
            let data = root.join(count.to_string());
            for i in 0..count {
                std::fs::create_dir_all(data.join(FILES).join(format!("{i:016x}"))).map_err(|e| e.to_string())?;
            }
            let cfg = Config { open_publish: true, data: data.clone(), ..Config::default() };
            cfg.check()?;
            let store = Store::open(&data, cfg.store())?;
            let expected = (count as usize).min(200);
            let problems = store.problems().to_vec();
            let no_loaded_files = store.is_empty();
            drop(store);
            if !problems.iter().all(|p| p.contains("meta.json")) || !no_loaded_files {
                return Err("unexpected diagnostic or loaded file".into());
            }
            if problems.len() != expected {
                return Err(format!("expected {expected} retained problems, got {}", problems.len()));
            }
            // The latest problems stay, in scan order: the first folder's is the one dropped.
            let first_kept = count as usize - expected;
            for (n, p) in problems.iter().enumerate() {
                let id = format!("{:016x}", first_kept + n);
                if !p.contains(&id) {
                    return Err(format!("problem {n} should be about {id}: {p}"));
                }
            }
        }
        Ok(())
    })();
    std::fs::remove_dir_all(&root).unwrap();
    assert!(result.is_ok(), "{}", result.unwrap_err());
}
