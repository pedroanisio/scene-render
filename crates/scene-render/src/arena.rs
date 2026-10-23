//! One malloc arena for the process, unless `MALLOC_ARENA_MAX` says otherwise.
//!
//! glibc gives each thread that allocates while others do an arena of its own, and an arena keeps what is freed in it
//! for its next allocations instead of returning it to the system. Mesa's software drivers allocate and free several
//! hundred megabytes on the thread that renders a large frame, and delivery renders on several threads, so the process
//! kept what each of them freed. Encoding the showcase films on lavapipe, one arena lowered peak RSS by 4-14 % (04-clay
//! 556 → 480 MiB) with wall time within run-to-run noise.

const M_ARENA_MAX: i32 = -8;

extern "C" {
    fn mallopt(param: i32, value: i32) -> i32;
}

/// Caps the arenas at one, unless the user set `MALLOC_ARENA_MAX`, which glibc has applied already.
#[allow(unsafe_code)]
extern "C" fn cap() {
    if std::env::var_os("MALLOC_ARENA_MAX").is_some() {
        return;
    }
    // SAFETY: mallopt takes two integers; it runs before main, with no other thread yet.
    unsafe { mallopt(M_ARENA_MAX, 1) };
}

/// Run by the loader before `main`, as every `.init_array` entry is: glibc no longer applies the cap once the process
/// has eight arenas, which opening a device on a software adapter gives it.
#[allow(unsafe_code)]
#[used]
#[link_section = ".init_array"]
static CAP: extern "C" fn() = cap;

#[cfg(test)]
mod tests {
    const NAME: &str = "delivery_threads_share_one_malloc_arena";

    /// Run alone in a child, four threads allocate at once; the child then reports its arenas.
    #[test]
    fn delivery_threads_share_one_malloc_arena() {
        if child_half() {
            return;
        }
        assert_eq!(sr_testkit::child_arenas(module_path!(), NAME), 1, "malloc arenas of four threads");
    }

    /// A cap the user set is theirs: two arenas for four threads.
    #[test]
    fn a_users_malloc_arena_max_stands() {
        const NAME: &str = "a_users_malloc_arena_max_stands";
        if child_half() {
            return;
        }
        let arenas = sr_testkit::child_arenas_with(module_path!(), NAME, &[("MALLOC_ARENA_MAX", "2")]);
        assert_eq!(arenas, 2, "malloc arenas of four threads under MALLOC_ARENA_MAX=2");
    }

    /// In the child: four threads allocate at once, then glibc reports the arenas.
    fn child_half() -> bool {
        if std::env::var_os(sr_testkit::ARENA_CHILD).is_none() {
            return false;
        }
        let all = std::sync::Barrier::new(4);
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    let block = std::hint::black_box(vec![1u8; 1 << 20]);
                    all.wait();
                    drop(block);
                });
            }
        });
        report_arenas();
        true
    }

    extern "C" {
        fn malloc_stats();
    }

    /// Writes glibc's report of the process's malloc arenas to stderr.
    #[allow(unsafe_code)]
    fn report_arenas() {
        // SAFETY: malloc_stats takes nothing and only reads the allocator's state.
        unsafe { malloc_stats() };
    }
}
