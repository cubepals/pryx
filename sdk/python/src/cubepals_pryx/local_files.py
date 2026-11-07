# SPDX-License-Identifier: FSL-1.1-ALv2
"""Local file and tar primitives refuse links and special entries; bodies stream in chunks."""
import hashlib,os,stat,tarfile
from pathlib import Path
from .generated.defaults import SDK_DEFAULTS

def local_path(path):return Path(os.path.abspath(path))
def directory(path,create=False):
    path=local_path(path);fd=os.open(path.anchor,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
    try:
        for part in path.parts[1:]:
            if create:
                try:os.mkdir(part,0o755,dir_fd=fd)
                except FileExistsError:pass
            child=os.open(part,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd)
            os.close(fd);fd=child
        return fd
    except BaseException:os.close(fd);raise

def open_file(path,write=False):
    path=local_path(path);parent=directory(path.parent,write)
    try:fd=os.open(path.name,(os.O_WRONLY|os.O_CREAT if write else os.O_RDONLY)|os.O_NOFOLLOW|os.O_NONBLOCK,0o600,dir_fd=parent)
    finally:os.close(parent)
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):raise ValueError('Expected a regular file')
        if write:os.ftruncate(fd,0)
        return os.fdopen(fd,'wb' if write else 'rb')
    except BaseException:os.close(fd);raise

def chunks(file,on_progress=None,total=None):
    size=0
    while part:=file.read(SDK_DEFAULTS['fileChunkBytes']):
        size+=len(part)
        if on_progress:on_progress(size,total)
        yield part

def entries(root,prefix=''):
    root=local_path(root);held=directory(root/prefix)
    try:
        for name in sorted(os.listdir(held)):
            relative=(Path(prefix)/name).as_posix();info=os.stat(name,dir_fd=held,follow_symlinks=False)
            if not stat.S_ISREG(info.st_mode) and not stat.S_ISDIR(info.st_mode):raise ValueError('Local transfers refuse links and special files')
            yield root/relative,relative,info
            if stat.S_ISDIR(info.st_mode):yield from entries(root,relative)
    finally:os.close(held)

def pack(root,file):
    with tarfile.open(fileobj=file,mode='w') as archive:
        size=0
        for count,(path,name,info) in enumerate(entries(root)):
            size+=info.st_size if stat.S_ISREG(info.st_mode) else 0
            if count>=SDK_DEFAULTS['archiveEntries'] or size+(count+1)*512+10240>SDK_DEFAULTS['archiveBytes']:raise ValueError('Archive exceeds packing budget')
            header=tarfile.TarInfo(name);header.mode=stat.S_IMODE(info.st_mode)&0o777
            if stat.S_ISDIR(info.st_mode):header.type=tarfile.DIRTYPE;archive.addfile(header)
            else:
                with open_file(path) as source:header.size=os.fstat(source.fileno()).st_size;archive.addfile(header,source)
    if file.tell()>SDK_DEFAULTS['archiveBytes']:raise ValueError('Archive exceeds packing budget')
    file.seek(0)

def extract(file,root):
    root=local_path(root);os.close(directory(root,True))
    with tarfile.open(fileobj=file,mode='r|') as archive:
        size=0;modes=[]
        for count,item in enumerate(archive):
            size+=item.size
            if count>=SDK_DEFAULTS['archiveEntries'] or size>SDK_DEFAULTS['archiveBytes']:raise ValueError('Archive exceeds extraction budget')
            if item.name.startswith('/') or '\\' in item.name or '..' in Path(item.name).parts or '\0' in item.name:raise ValueError('Unsafe archive path')
            path=local_path(root/item.name)
            if item.isdir():
                os.close(directory(path,True));modes.append((path,item.mode&0o777))
            elif item.isfile() and not item.issparse():
                with archive.extractfile(item) as source,open_file(path,True) as destination:
                    for part in chunks(source):destination.write(part)
                    os.fchmod(destination.fileno(),item.mode&0o777)
            else:raise ValueError('Archive links and special files are refused')
        for path,mode in reversed(modes):
            held=directory(path)
            try:os.fchmod(held,mode)
            finally:os.close(held)

def digest(parts):
    value=hashlib.sha256()
    for part in parts:value.update(part)
    return value.hexdigest()
