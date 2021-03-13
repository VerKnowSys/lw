# LogWatcher

LogWatcher is an utility meant as a replacement for: `tail -F /some/path/*`.

lw vs tail advantages:

 - it watches for modified, deleted and _new_ files without restart of the utility
 - it won't crash if there are > 4096 files (shell pattern limit exhaustion) or directories (if you set `ulimit -n` value high enough)
 - it works recursively on directories


# Author:

Daniel ([@dmilith](https://twitter.com/dmilith)) Dettlaff



# Features:

- Uses Kqueue for event monitoring (standard on BSD and macOS)

- Works recursively through files/ directories but can be also used for single file monitoring

- It's fast and DEBUG'able (through DEBUG and TRACE env variables)

- Produces colorful output (especially in DEBUG and TRACE mode).


## Installation:

```sh
cargo install --force lw
```



## Software requirements:

- Rust >= 1.40.0



## Additional build requirements:

- Clang >= 10.x
- Make >= 3.x
- Cmake >= 3.16
- POSIX compliant base-system (tested on systems: FreeBSD/ HardenedBSD/ Darwin)



## License

- BSD

- MIT

