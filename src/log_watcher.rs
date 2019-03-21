use kqueue_sys::*;
use kqueue::*;
use std::env;
use std::path::Path;
use walkdir::WalkDir;
use std::process::exit;
use std::fmt::Display;
use chrono::Local;


fn fatal<S: Display>(fmt: S) -> ! {
    println!("ERROR: {}", fmt);
    exit(1);
}


// let selfpid = unsafe { getpid() };
// NOTE: this doesn't work on Darwin. Causes: ERROR: kqueue failed: Operation not supported (os error 45):
//
// kqueue_watcher
//     .add_pid(
//         selfpid,
//         EventFilter::EVFILT_PROC,
//         NOTE_EXIT | NOTE_FORK | NOTE_EXEC | NOTE_TRACK | NOTE_TRACKERR
//     )
//     .unwrap_or_else(|e| fatal(format!("Could not watch pid {}: {}", selfpid, e)));


fn main() {
    let mut kqueue_watcher = Watcher::new().unwrap_or_else(|e| fatal(format!("Could not create kq watcher: {}", e)));
    let paths_arguments: Vec<_>
        = env::args()
            .skip(1) // first arg is $0
            .collect();

    println!("Watching paths: {}", paths_arguments.join(", "));
    for path in paths_arguments {
        let file_path = Path::new(&path);
        watch_file(&mut kqueue_watcher, &file_path);
        let walk_dir = WalkDir::new(file_path)
                      .follow_links(true)
                      .min_depth(1)
                      .max_depth(1)
                      .into_iter();

        // filter out only readable/accessible/proper files:
        walk_dir
            .filter_map(|element| element.ok())
            .for_each(|element| watch_file(&mut kqueue_watcher, element.path()));

        // Recursively watch whole path if a directory:
        kqueue_watcher
            .watch()
            .unwrap_or_else(|error_cause| fatal(format!("kqueue failed: {}", error_cause)));
    }

    // handle events:
    for kqueue_event in kqueue_watcher.iter() {
        println!("EVENT: TS: {}: {:?}", Local::now().to_rfc3339(), kqueue_event, );
    }
}


fn watch_file(kqueue_watcher: &mut Watcher, file: &Path) {
    kqueue_watcher
        .add_filename(
            &file,
            EventFilter::EVFILT_VNODE, // NOTE: no NOTE_TRUNCATE on Darwin + ignore on NOTE_ATTRIB and NOTE_EXTEND
            NOTE_DELETE | NOTE_WRITE |
            NOTE_LINK | NOTE_RENAME | NOTE_REVOKE
        )
        .unwrap_or_else(|error_cause| fatal(format!("Could not watch file {:?}: {}", file, error_cause)));
}
