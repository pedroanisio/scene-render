//! A script `executable` writes runs at once while other threads of the same process write and fork theirs: none of
//! them fails with "Text file busy" (ETXTBSY), which a script written with `std::fs::write` and run straight after
//! does, intermittently, when another test thread forks while the write's descriptor is open.

#![cfg(unix)]

use std::process::Command;

#[test]
fn scripts_run_at_once_from_many_threads() {
    let dir = std::env::temp_dir().join(format!("sr-testkit-executable-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                for i in 0..25 {
                    let path = dir.join(format!("script-{t}-{i}"));
                    sr_testkit::executable(&path, &format!("#!/bin/sh\necho {t}-{i}\n"));
                    let out = Command::new(&path).output().unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                    assert!(out.status.success(), "{}: {:?}", path.display(), out);
                    assert_eq!(String::from_utf8_lossy(&out.stdout), format!("{t}-{i}\n"));
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert!(left.iter().all(|n| !n.to_string_lossy().ends_with(".src")), "staging files left behind: {left:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_existing_script_is_replaced() {
    let dir = std::env::temp_dir().join(format!("sr-testkit-replace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tool");
    sr_testkit::executable(&path, "#!/bin/sh\nexit 3\n");
    sr_testkit::executable(&path, "#!/bin/sh\nexit 0\n");
    assert!(Command::new(&path).status().unwrap().success());
    std::fs::remove_dir_all(&dir).unwrap();
}
