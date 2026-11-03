// Game data is read only. Only /user is writable; this overlay is private to a worker.
export class BrowserFiles {
  constructor(readBlob) {
    this.files = new Map();
    this.directories = new Set(['/','/game','/user']);
    this.overlay = new Map();
    this.readBlob = readBlob;
    this.dirty = false;
  }
  path(value) {
    if (typeof value !== 'string' || !value.startsWith('/') || value.includes('\\') || value.includes('\0')) throw Error('Invalid virtual path');
    const parts = value.split('/').filter(Boolean);
    if (parts.some(p => p === '.' || p === '..' || p.includes(':'))) throw Error('Invalid virtual path component');
    return '/' + parts.join('/').toLowerCase();
  }
  parents(path) {
    const parts = path.split('/');
    for (let i = 1; i < parts.length; i++) this.directories.add(parts.slice(0, i).join('/') || '/');
  }
  mount(relative, blob) {
    const path = this.path('/game/' + relative);
    if (this.files.has(path)) throw Error('Duplicate file after case folding: ' + relative);
    if (blob.size > 0xffffffff) throw Error('Files above 4 GiB are unsupported');
    this.parents(path);
    this.files.set(path, blob);
  }
  writable(path) {
    if (!path.startsWith('/user/')) throw Error('Game data is read only');
  }
  size = path => {
    path = this.path(path);
    if (this.overlay.has(path)) return this.overlay.get(path).length;
    if (this.files.has(path)) return this.files.get(path).size;
    return this.directories.has(path) ? -2 : -1;
  };
  read = (path, offset, length) => {
    path = this.path(path);
    if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(length) || length < 0) throw Error('Invalid read range');
    const bytes = this.overlay.get(path);
    if (bytes) return bytes.slice(offset, offset + length);
    const blob = this.files.get(path);
    if (!blob) throw Error('File not found: ' + path);
    return this.readBlob(blob.slice(offset, offset + length));
  };
  write = (path, offset, bytes) => {
    path = this.path(path); this.writable(path);
    const old = this.overlay.get(path);
    if (!old) throw Error('Writable file is not open');
    const end = offset + bytes.length;
    if (!Number.isSafeInteger(offset) || offset < 0 || end > 256 * 1024 * 1024) throw Error('Save exceeds 256 MiB limit');
    const result = new Uint8Array(Math.max(old.length, end));
    result.set(old); result.set(bytes, offset);
    this.overlay.set(path, result); this.dirty = true;
  };
  operation = (op, path, target) => {
    path = this.path(path);
    if (op === 'mkdir') {
      if (path !== '/user') this.writable(path);
      this.parents(path + '/'); this.directories.add(path); return;
    }
    this.writable(path);
    if (op === 'truncate') {
      const parent = path.slice(0, path.lastIndexOf('/'));
      if (!this.directories.has(parent)) throw Error('Parent directory missing');
      this.overlay.set(path, new Uint8Array());
    } else if (op === 'remove') {
      if (!this.overlay.delete(path)) throw Error('File not found');
    } else if (op === 'rename') {
      target = this.path(target); this.writable(target);
      if (!this.overlay.has(path)) throw Error('File not found');
      this.overlay.set(target, this.overlay.get(path)); this.overlay.delete(path);
    } else throw Error('Unsupported filesystem operation');
    this.dirty = true;
  };
  snapshot() {
    return [...this.overlay].filter(([path]) => !path.endsWith('.lock') && !path.endsWith('.tmp'));
  }
  restore(entries) {
    if (!Array.isArray(entries)) throw Error('Invalid save archive');
    const restored = new Map(); let total = 0;
    for (const [name, value] of entries) {
      const path = this.path(name); this.writable(path);
      if (!(value instanceof Uint8Array)) throw Error('Invalid save bytes');
      total += value.length;
      if (total > 256 * 1024 * 1024) throw Error('Save archive exceeds 256 MiB');
      if (restored.has(path)) throw Error('Duplicate save entry');
      restored.set(path, value);
    }
    this.overlay = restored;
    for (const path of restored.keys()) this.parents(path);
  }
}
