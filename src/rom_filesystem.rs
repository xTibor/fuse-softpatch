use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;

use fuse_mt::{DirectoryEntry, FileAttr, FileType, FilesystemMT, RequestInfo};
use fuse_mt::{ResultEmpty, ResultEntry, ResultOpen, ResultReaddir};
use time::Timespec;

use crate::bps::BpsHeader;
use crate::rom_manager::RomManager;

const EPOCH: Timespec = Timespec { sec: 0, nsec: 0 };
const TTL: Timespec = Timespec { sec: 1, nsec: 0 };

fn timespec_from(st: &SystemTime) -> Timespec {
    if let Ok(dur_since_epoch) = st.duration_since(std::time::UNIX_EPOCH) {
        Timespec::new(dur_since_epoch.as_secs() as i64, dur_since_epoch.subsec_nanos() as i32)
    } else {
        Timespec::new(0, 0)
    }
}

enum Handle {
    Directory { attr: FileAttr },
    File { attr: FileAttr, data: Option<Vec<u8>> },
}

pub struct RomFilesystem {
    rom_manager: Mutex<RomManager>,
    handles: Mutex<HashMap<u64, Handle>>,
    next_handle: Mutex<u64>,
}

impl RomFilesystem {
    pub fn new(rom_manager: RomManager) -> Self {
        Self {
            rom_manager: Mutex::new(rom_manager),
            handles: Mutex::new(HashMap::new()),
            next_handle: Mutex::new(1),
        }
    }

    fn get_root_attr(&self) -> FileAttr {
        FileAttr {
            size: 0,
            blocks: 0,
            atime: EPOCH,
            mtime: EPOCH,
            ctime: EPOCH,
            crtime: EPOCH,
            kind: FileType::Directory,
            perm: 0o444,
            nlink: 1,
            uid: unsafe { libc::geteuid() },
            gid: unsafe { libc::getegid() },
            rdev: 0,
            flags: 0,
        }
    }

    fn get_file_attr(&self, header: &BpsHeader) -> FileAttr {
        FileAttr {
            size: header.target_size,
            blocks: 0,
            atime: timespec_from(&header.access_time),
            mtime: timespec_from(&header.modify_time),
            ctime: timespec_from(&header.modify_time),
            crtime: timespec_from(&header.create_time),
            kind: FileType::RegularFile,
            perm: 0o444,
            nlink: 1,
            uid: unsafe { libc::geteuid() },
            gid: unsafe { libc::getegid() },
            rdev: 0,
            flags: 0,
        }
    }
}

impl FilesystemMT for RomFilesystem {
    fn init(&self, _req: RequestInfo) -> ResultEmpty {
        eprintln!("init");

        Ok(())
    }

    fn opendir(&self, _req: RequestInfo, path: &Path, _flags: u32) -> ResultOpen {
        let path = path.strip_prefix("/").unwrap();
        let mut handles = self.handles.lock().unwrap();
        let mut next_handle = self.next_handle.lock().unwrap();

        eprintln!("opendir: {:?}", path);

        if path == Path::new("") {
            let handle = *next_handle;
            *next_handle += 1;

            handles.insert(
                handle,
                Handle::Directory {
                    attr: self.get_root_attr(),
                },
            );
            Ok((handle, 0))
        } else {
            Err(libc::ENOENT)
        }
    }

    fn readdir(&self, _req: RequestInfo, path: &Path, fh: u64) -> ResultReaddir {
        let path = path.strip_prefix("/").unwrap();
        let rom_manager = self.rom_manager.lock().unwrap();
        let handles = self.handles.lock().unwrap();

        eprintln!("readdir: {:?}", path);

        if let Some(Handle::Directory { .. }) = handles.get(&fh) {
            let mut files = Vec::new();

            files.push(DirectoryEntry {
                name: ".".into(),
                kind: FileType::Directory,
            });

            files.push(DirectoryEntry {
                name: "..".into(),
                kind: FileType::Directory,
            });

            for path in rom_manager.target_roms.keys() {
                files.push(DirectoryEntry {
                    name: path.into(),
                    kind: FileType::RegularFile,
                });
            }

            Ok(files)
        } else {
            Err(libc::ENOENT)
        }
    }

    fn releasedir(&self, _req: RequestInfo, path: &Path, fh: u64, _flags: u32) -> ResultEmpty {
        let path = path.strip_prefix("/").unwrap();
        let mut handles = self.handles.lock().unwrap();

        eprintln!("releasedir: {:?}", path);

        if let Some(Handle::Directory { .. }) = handles.get(&fh) {
            handles.remove(&fh);
            Ok(())
        } else {
            Err(libc::ENOENT)
        }
    }

    fn access(&self, _req: RequestInfo, path: &Path, mask: u32) -> ResultEmpty {
        let path = path.strip_prefix("/").unwrap();

        eprintln!("access: {:?}", path);

        Ok(())
    }

    #[allow(clippy::collapsible_if)]
    fn getattr(&self, _req: RequestInfo, path: &Path, fh: Option<u64>) -> ResultEntry {
        let path = path.strip_prefix("/").unwrap();
        let rom_manager = self.rom_manager.lock().unwrap();
        let handles = self.handles.lock().unwrap();

        eprintln!("getattr: {:?}", path);

        if let Some(fh) = fh {
            match handles.get(&fh) {
                Some(Handle::Directory { attr }) => Ok((TTL, *attr)),
                Some(Handle::File { attr, .. }) => Ok((TTL, *attr)),
                _ => Err(libc::ENOENT),
            }
        } else {
            if path == Path::new("") {
                Ok((TTL, self.get_root_attr()))
            } else if let Some(rom) = rom_manager.target_roms.get(path) {
                Ok((TTL, self.get_file_attr(rom)))
            } else {
                Err(libc::ENOENT)
            }
        }
    }

    fn open(&self, _req: RequestInfo, path: &Path, _flags: u32) -> ResultOpen {
        let path = path.strip_prefix("/").unwrap();
        let rom_manager = self.rom_manager.lock().unwrap();
        let mut handles = self.handles.lock().unwrap();
        let mut next_handle = self.next_handle.lock().unwrap();

        eprintln!("open: {:?}", path);

        if let Some(rom) = rom_manager.target_roms.get(path) {
            let handle = *next_handle;
            *next_handle += 1;

            handles.insert(
                handle,
                Handle::File {
                    attr: self.get_file_attr(&rom),
                    data: None,
                },
            );

            Ok((handle, 0))
        } else {
            Err(libc::ENOENT)
        }
    }

    fn read(
        &self,
        _req: RequestInfo,
        _path: &Path,
        fh: u64,
        offset: u64,
        size: u32,
        result: impl FnOnce(std::result::Result<&[u8], libc::c_int>),
    ) {
        // TODO: Deferred ROM patching on read
        // if is_none() { rom.generate_patched_rom() }

        result(Err(libc::ENOSYS))
    }

    fn release(
        &self,
        _req: RequestInfo,
        path: &Path,
        fh: u64,
        _flags: u32,
        _lock_owner: u64,
        _flush: bool,
    ) -> ResultEmpty {
        let path = path.strip_prefix("/").unwrap();
        let mut handles = self.handles.lock().unwrap();

        eprintln!("release: {:?}", path);

        if let Some(Handle::File { .. }) = handles.get(&fh) {
            handles.remove(&fh);
            Ok(())
        } else {
            Err(libc::ENOENT)
        }
    }
}
