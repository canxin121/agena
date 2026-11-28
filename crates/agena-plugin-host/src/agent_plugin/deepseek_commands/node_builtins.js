(() => {
    const MAX_BUFFER_BYTES = 8 * 1024 * 1024;
    const B64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    const B64_TABLE = new Int16Array(128).fill(-1);
    for (let i = 0; i < B64_ALPHABET.length; i++) B64_TABLE[B64_ALPHABET.charCodeAt(i)] = i;

    function encodeUtf8(str) {
        const s = String(str);
        const out = [];
        for (let i = 0; i < s.length; i++) {
            let code = s.charCodeAt(i);
            if (code >= 0xd800 && code <= 0xdbff && i + 1 < s.length) {
                const next = s.charCodeAt(i + 1);
                if (next >= 0xdc00 && next <= 0xdfff) {
                    code = ((code - 0xd800) << 10) + (next - 0xdc00) + 0x10000;
                    i++;
                }
            }
            if (code < 0x80) out.push(code);
            else if (code < 0x800) out.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
            else if (code < 0x10000) out.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
            else out.push(0xf0 | (code >> 18), 0x80 | ((code >> 12) & 0x3f), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
        }
        return new Uint8Array(out);
    }

    function decodeUtf8(bytes) {
        let out = '';
        let i = 0;
        const len = bytes.length;
        while (i < len) {
            const b0 = bytes[i++];
            if (b0 < 0x80) {
                out += String.fromCharCode(b0);
            } else if ((b0 & 0xe0) === 0xc0 && i < len) {
                const b1 = bytes[i++];
                if ((b1 & 0xc0) !== 0x80) { out += '\ufffd'; i--; continue; }
                const cp = ((b0 & 0x1f) << 6) | (b1 & 0x3f);
                out += cp < 0x80 ? '\ufffd' : String.fromCharCode(cp);
            } else if ((b0 & 0xf0) === 0xe0 && i + 1 < len) {
                const b1 = bytes[i++], b2 = bytes[i++];
                if ((b1 & 0xc0) !== 0x80 || (b2 & 0xc0) !== 0x80) { out += '\ufffd'; continue; }
                const cp = ((b0 & 0x0f) << 12) | ((b1 & 0x3f) << 6) | (b2 & 0x3f);
                out += (cp < 0x800 || (cp >= 0xd800 && cp <= 0xdfff)) ? '\ufffd' : String.fromCharCode(cp);
            } else if ((b0 & 0xf8) === 0xf0 && i + 2 < len) {
                const b1 = bytes[i++], b2 = bytes[i++], b3 = bytes[i++];
                if ((b1 & 0xc0) !== 0x80 || (b2 & 0xc0) !== 0x80 || (b3 & 0xc0) !== 0x80) { out += '\ufffd'; continue; }
                const cp = ((b0 & 0x07) << 18) | ((b1 & 0x3f) << 12) | ((b2 & 0x3f) << 6) | (b3 & 0x3f);
                if (cp < 0x10000 || cp > 0x10ffff) { out += '\ufffd'; continue; }
                const adj = cp - 0x10000;
                out += String.fromCharCode(0xd800 | (adj >> 10), 0xdc00 | (adj & 0x3ff));
            } else {
                out += '\ufffd';
            }
        }
        return out;
    }

    function encodeBase64(bytes, urlSafe = false) {
        let out = '';
        for (let i = 0; i < bytes.length; i += 3) {
            const b0 = bytes[i];
            const has1 = i + 1 < bytes.length;
            const has2 = i + 2 < bytes.length;
            const b1 = has1 ? bytes[i + 1] : 0;
            const b2 = has2 ? bytes[i + 2] : 0;
            out += B64_ALPHABET[b0 >> 2];
            out += B64_ALPHABET[((b0 & 3) << 4) | (b1 >> 4)];
            out += has1 ? B64_ALPHABET[((b1 & 15) << 2) | (b2 >> 6)] : (urlSafe ? '' : '=');
            out += has2 ? B64_ALPHABET[b2 & 63] : (urlSafe ? '' : '=');
        }
        if (urlSafe) out = out.replace(/\+/g, '-').replace(/\//g, '_');
        return out;
    }

    function decodeBase64(input) {
        let clean = String(input).replace(/[\s]/g, '').replace(/-/g, '+').replace(/_/g, '/');
        while (clean.length % 4 === 2 || clean.length % 4 === 3) clean += '=';
        if (clean.length % 4 !== 0) throw new TypeError('Invalid base64 string');
        const out = [];
        for (let i = 0; i < clean.length; i += 4) {
            const c0 = clean.charCodeAt(i), c1 = clean.charCodeAt(i + 1);
            const c2 = clean.charCodeAt(i + 2), c3 = clean.charCodeAt(i + 3);
            const v0 = c0 < 128 ? B64_TABLE[c0] : -1;
            const v1 = c1 < 128 ? B64_TABLE[c1] : -1;
            const pad2 = c2 === 61, pad3 = c3 === 61;
            const v2 = pad2 ? 0 : (c2 < 128 ? B64_TABLE[c2] : -1);
            const v3 = pad3 ? 0 : (c3 < 128 ? B64_TABLE[c3] : -1);
            if (v0 < 0 || v1 < 0 || v2 < 0 || v3 < 0 || (pad2 && !pad3)) {
                throw new TypeError('Invalid base64 string');
            }
            out.push((v0 << 2) | (v1 >> 4));
            if (!pad2) out.push(((v1 & 15) << 4) | (v2 >> 2));
            if (!pad3) out.push(((v2 & 3) << 6) | v3);
        }
        return new Uint8Array(out);
    }

    function encodeHex(bytes) {
        let out = '';
        for (let i = 0; i < bytes.length; i++) {
            out += bytes[i].toString(16).padStart(2, '0');
        }
        return out;
    }

    function decodeHex(input) {
        const s = String(input);
        const len = Math.floor(s.length / 2);
        const out = new Uint8Array(len);
        for (let i = 0; i < len; i++) {
            const byte = parseInt(s.slice(i * 2, i * 2 + 2), 16);
            if (Number.isNaN(byte)) return out.subarray(0, i);
            out[i] = byte;
        }
        return out;
    }

    function encodeLatin1(str) {
        const s = String(str);
        const out = new Uint8Array(s.length);
        for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i) & 0xff;
        return out;
    }

    function decodeLatin1(bytes) {
        let out = '';
        for (let i = 0; i < bytes.length; i++) out += String.fromCharCode(bytes[i]);
        return out;
    }

    function encodeString(str, encoding = 'utf8') {
        const enc = String(encoding).toLowerCase();
        if (enc === 'utf8' || enc === 'utf-8') return encodeUtf8(str);
        if (enc === 'base64' || enc === 'base64url') return decodeBase64(str);
        if (enc === 'hex') return decodeHex(str);
        if (enc === 'latin1' || enc === 'binary' || enc === 'ascii') return encodeLatin1(str);
        throw new TypeError(`Unknown encoding: ${encoding}`);
    }

    function decodeBytes(bytes, encoding = 'utf8') {
        const enc = String(encoding).toLowerCase();
        if (enc === 'utf8' || enc === 'utf-8') return decodeUtf8(bytes);
        if (enc === 'base64') return encodeBase64(bytes, false);
        if (enc === 'base64url') return encodeBase64(bytes, true);
        if (enc === 'hex') return encodeHex(bytes);
        if (enc === 'latin1' || enc === 'binary' || enc === 'ascii') return decodeLatin1(bytes);
        throw new TypeError(`Unknown encoding: ${encoding}`);
    }

    class TextEncoder {
        get encoding() { return 'utf-8'; }
        encode(input = '') { return encodeUtf8(input); }
    }

    class TextDecoder {
        constructor(label = 'utf-8') {
            const normalized = String(label).trim().toLowerCase();
            if (normalized !== 'utf-8' && normalized !== 'utf8') {
                throw new RangeError(`Unsupported TextDecoder encoding: ${label}`);
            }
            this.encoding = 'utf-8';
        }
        decode(input) {
            if (input === undefined || input === null) return '';
            if (input instanceof ArrayBuffer) return decodeUtf8(new Uint8Array(input));
            if (ArrayBuffer.isView(input)) {
                return decodeUtf8(new Uint8Array(input.buffer, input.byteOffset, input.byteLength));
            }
            throw new TypeError('TextDecoder.decode expects an ArrayBuffer or view');
        }
    }

    class Buffer extends Uint8Array {
        static isBuffer(value) { return value instanceof Buffer; }
        static from(value, encodingOrOffset, length) {
            if (typeof value === 'string') {
                const bytes = encodeString(value, encodingOrOffset ?? 'utf8');
                if (bytes.length > MAX_BUFFER_BYTES) throw new RangeError('Buffer exceeds 8 MiB limit');
                const buf = new Buffer(bytes.length);
                buf.set(bytes);
                return buf;
            }
            if (value instanceof ArrayBuffer) {
                const offset = encodingOrOffset ?? 0;
                const view = length === undefined
                    ? new Uint8Array(value, offset)
                    : new Uint8Array(value, offset, length);
                if (view.length > MAX_BUFFER_BYTES) throw new RangeError('Buffer exceeds 8 MiB limit');
                const buf = new Buffer(view.length);
                buf.set(view);
                return buf;
            }
            if (ArrayBuffer.isView(value)) {
                if (value.byteLength > MAX_BUFFER_BYTES) throw new RangeError('Buffer exceeds 8 MiB limit');
                const view = new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
                const buf = new Buffer(view.length);
                buf.set(view);
                return buf;
            }
            if (Array.isArray(value)) {
                if (value.length > MAX_BUFFER_BYTES) throw new RangeError('Buffer exceeds 8 MiB limit');
                const buf = new Buffer(value.length);
                for (let i = 0; i < value.length; i++) buf[i] = Number(value[i]) & 0xff;
                return buf;
            }
            throw new TypeError('Unsupported Buffer.from input');
        }
        static alloc(size, fill, encoding) {
            if (!Number.isSafeInteger(size) || size < 0 || size > MAX_BUFFER_BYTES) {
                throw new RangeError('Invalid Buffer.alloc size');
            }
            const buf = new Buffer(size);
            if (fill !== undefined && size > 0) {
                if (typeof fill === 'number') {
                    buf.fill(fill & 0xff);
                } else {
                    const pattern = typeof fill === 'string'
                        ? encodeString(fill, encoding ?? 'utf8')
                        : Buffer.from(fill);
                    if (pattern.length > 0) {
                        for (let i = 0; i < size; i++) buf[i] = pattern[i % pattern.length];
                    }
                }
            }
            return buf;
        }
        static allocUnsafe(size) { return Buffer.alloc(size); }
        static byteLength(value, encoding = 'utf8') {
            if (typeof value === 'string') return encodeString(value, encoding).length;
            if (ArrayBuffer.isView(value) || value instanceof ArrayBuffer) return value.byteLength;
            throw new TypeError('Unsupported Buffer.byteLength input');
        }
        static concat(list, totalLength) {
            if (!Array.isArray(list)) throw new TypeError('Buffer.concat requires an array');
            let total = 0;
            for (const item of list) {
                if (!ArrayBuffer.isView(item)) throw new TypeError('Buffer.concat items must be Uint8Array');
                total += item.byteLength;
            }
            const length = totalLength === undefined ? total : totalLength;
            if (!Number.isSafeInteger(length) || length < 0 || length > MAX_BUFFER_BYTES) {
                throw new RangeError('Invalid Buffer.concat length');
            }
            const out = new Buffer(length);
            let offset = 0;
            for (const item of list) {
                if (offset >= length) break;
                const view = new Uint8Array(item.buffer, item.byteOffset, Math.min(item.byteLength, length - offset));
                out.set(view, offset);
                offset += view.length;
            }
            return out;
        }
        toString(encoding = 'utf8', start = 0, end = this.length) {
            const slice = this.subarray(start, end);
            return decodeBytes(slice, encoding);
        }
        equals(other) {
            if (!ArrayBuffer.isView(other)) throw new TypeError('Buffer.equals requires a Uint8Array');
            if (this.byteLength !== other.byteLength) return false;
            const rhs = new Uint8Array(other.buffer, other.byteOffset, other.byteLength);
            for (let i = 0; i < this.length; i++) if (this[i] !== rhs[i]) return false;
            return true;
        }
        subarray(start, end) {
            const sub = super.subarray(start, end);
            return Object.setPrototypeOf(sub, Buffer.prototype);
        }
        slice(start, end) { return this.subarray(start, end); }
    }

    class URLSearchParams {
        constructor(init) {
            this._entries = [];
            if (typeof init === 'string') {
                const raw = init.startsWith('?') ? init.slice(1) : init;
                if (raw) {
                    for (const part of raw.split('&')) {
                        if (!part) continue;
                        const eq = part.indexOf('=');
                        const k = eq >= 0 ? part.slice(0, eq) : part;
                        const v = eq >= 0 ? part.slice(eq + 1) : '';
                        this._entries.push([
                            decodeURIComponent(k.replace(/\+/g, ' ')),
                            decodeURIComponent(v.replace(/\+/g, ' ')),
                        ]);
                    }
                }
            } else if (Array.isArray(init)) {
                for (const pair of init) {
                    if (!Array.isArray(pair) || pair.length !== 2) throw new TypeError('Invalid URLSearchParams pair');
                    this._entries.push([String(pair[0]), String(pair[1])]);
                }
            } else if (init instanceof URLSearchParams) {
                this._entries = init._entries.map(([k, v]) => [k, v]);
            } else if (init && typeof init === 'object') {
                for (const [k, v] of Object.entries(init)) this._entries.push([String(k), String(v)]);
            }
        }
        append(name, value) { this._entries.push([String(name), String(value)]); }
        delete(name) { const k = String(name); this._entries = this._entries.filter(([n]) => n !== k); }
        get(name) { const k = String(name); const found = this._entries.find(([n]) => n === k); return found ? found[1] : null; }
        getAll(name) { const k = String(name); return this._entries.filter(([n]) => n === k).map(([, v]) => v); }
        has(name) { const k = String(name); return this._entries.some(([n]) => n === k); }
        set(name, value) {
            const k = String(name), v = String(value);
            let replaced = false;
            this._entries = this._entries.filter(([n], i, arr) => {
                if (n !== k) return true;
                if (!replaced) { arr[i] = [k, v]; replaced = true; return true; }
                return false;
            });
            if (!replaced) this._entries.push([k, v]);
        }
        sort() { this._entries.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)); }
        entries() { return this._entries.map(([k, v]) => [k, v])[Symbol.iterator](); }
        keys() { return this._entries.map(([k]) => k)[Symbol.iterator](); }
        values() { return this._entries.map(([, v]) => v)[Symbol.iterator](); }
        forEach(cb, thisArg) { for (const [k, v] of this._entries) cb.call(thisArg, v, k, this); }
        [Symbol.iterator]() { return this.entries(); }
        get size() { return this._entries.length; }
        toString() {
            return this._entries
                .map(([k, v]) => `${encodeURIComponent(k).replace(/%20/g, '+')}=${encodeURIComponent(v).replace(/%20/g, '+')}`)
                .join('&');
        }
    }

    function assertPath(value) {
        if (typeof value !== 'string') throw new TypeError('Path must be a string');
        return value;
    }

    function normalizeStringPosix(path, allowAboveRoot) {
        let res = '';
        let lastSegmentLength = 0;
        let lastSlash = -1;
        let dots = 0;
        let code = 0;
        for (let i = 0; i <= path.length; ++i) {
            if (i < path.length) code = path.charCodeAt(i);
            else if (code === 47) break;
            else code = 47;
            if (code === 47) {
                if (lastSlash === i - 1 || dots === 1) {
                    // noop
                } else if (dots === 2) {
                    if (res.length < 2 || lastSegmentLength !== 2 || res.charCodeAt(res.length - 1) !== 46 || res.charCodeAt(res.length - 2) !== 46) {
                        if (res.length > 2) {
                            const lastSlashIndex = res.lastIndexOf('/');
                            if (lastSlashIndex === -1) { res = ''; lastSegmentLength = 0; }
                            else { res = res.slice(0, lastSlashIndex); lastSegmentLength = res.length - 1 - res.lastIndexOf('/'); }
                            lastSlash = i; dots = 0; continue;
                        } else if (res.length !== 0) {
                            res = ''; lastSegmentLength = 0; lastSlash = i; dots = 0; continue;
                        }
                    }
                    if (allowAboveRoot) {
                        res += res.length > 0 ? '/..' : '..';
                        lastSegmentLength = 2;
                    }
                } else {
                    if (res.length > 0) res += '/' + path.slice(lastSlash + 1, i);
                    else res = path.slice(lastSlash + 1, i);
                    lastSegmentLength = i - lastSlash - 1;
                }
                lastSlash = i;
                dots = 0;
            } else if (code === 46 && dots !== -1) {
                ++dots;
            } else {
                dots = -1;
            }
        }
        return res;
    }

    const posix = {
        sep: '/',
        delimiter: ':',
        isAbsolute(path) { return assertPath(path).length > 0 && path.charCodeAt(0) === 47; },
        normalize(path) {
            assertPath(path);
            if (!path.length) return '.';
            const isAbs = path.charCodeAt(0) === 47;
            const trailingSlash = path.charCodeAt(path.length - 1) === 47;
            let res = normalizeStringPosix(path, !isAbs);
            if (!res.length) {
                if (isAbs) return '/';
                return trailingSlash ? './' : '.';
            }
            if (trailingSlash) res += '/';
            return isAbs ? '/' + res : res;
        },
        join(...args) {
            if (!args.length) return '.';
            let joined;
            for (const arg of args) {
                assertPath(arg);
                if (arg.length > 0) joined = joined === undefined ? arg : joined + '/' + arg;
            }
            return joined === undefined ? '.' : posix.normalize(joined);
        },
        resolve(...args) {
            let resolvedPath = '';
            let resolvedAbsolute = false;
            for (let i = args.length - 1; i >= 0 && !resolvedAbsolute; i--) {
                const path = assertPath(args[i]);
                if (!path.length) continue;
                resolvedPath = path + '/' + resolvedPath;
                resolvedAbsolute = path.charCodeAt(0) === 47;
            }
            resolvedPath = normalizeStringPosix(resolvedPath, !resolvedAbsolute);
            if (resolvedAbsolute) return '/' + resolvedPath;
            return resolvedPath.length > 0 ? resolvedPath : '.';
        },
        dirname(path) {
            assertPath(path);
            if (!path.length) return '.';
            const hasRoot = path.charCodeAt(0) === 47;
            let end = -1;
            let matchedSlash = true;
            for (let i = path.length - 1; i >= 1; --i) {
                if (path.charCodeAt(i) === 47) {
                    if (!matchedSlash) { end = i; break; }
                } else {
                    matchedSlash = false;
                }
            }
            if (end === -1) return hasRoot ? '/' : '.';
            if (hasRoot && end === 1) return '//';
            return path.slice(0, end);
        },
        basename(path, suffix) {
            assertPath(path);
            if (suffix !== undefined && typeof suffix !== 'string') throw new TypeError('Suffix must be a string');
            let end = path.length;
            while (end > 0 && path.charCodeAt(end - 1) === 47) end--;
            if (end === 0) return '';
            let start = end - 1;
            while (start >= 0 && path.charCodeAt(start) !== 47) start--;
            const base = path.slice(start + 1, end);
            if (suffix && base.endsWith(suffix)) return base.slice(0, base.length - suffix.length);
            return base;
        },
        extname(path) {
            const base = posix.basename(path);
            const dot = base.lastIndexOf('.');
            return dot <= 0 ? '' : base.slice(dot);
        },
        relative(from, to) {
            const f = posix.resolve(assertPath(from));
            const t = posix.resolve(assertPath(to));
            if (f === t) return '';
            const fromParts = f.split('/').filter(Boolean);
            const toParts = t.split('/').filter(Boolean);
            let common = 0;
            while (common < fromParts.length && common < toParts.length && fromParts[common] === toParts[common]) {
                common++;
            }
            const up = new Array(fromParts.length - common).fill('..');
            return up.concat(toParts.slice(common)).join('/');
        },
        parse(path) {
            assertPath(path);
            const root = path.startsWith('/') ? '/' : '';
            const base = posix.basename(path);
            const ext = posix.extname(base);
            const name = ext ? base.slice(0, -ext.length) : base;
            const dir = path ? posix.dirname(path) : '';
            return { root, dir: path.includes('/') ? dir : '', base, ext, name };
        },
        format(obj) {
            if (!obj || typeof obj !== 'object') throw new TypeError('Path object must be an object');
            const dir = obj.dir || obj.root || '';
            const base = obj.base || `${obj.name || ''}${obj.ext || ''}`;
            if (!dir) return base;
            return dir === obj.root ? `${dir}${base}` : `${dir}/${base}`;
        },
    };
    posix.posix = posix;
    Object.freeze(posix);

    class EventEmitter {
        constructor() {
            this._events = new Map();
            this._maxListeners = 10;
        }
        setMaxListeners(n) {
            if (typeof n !== 'number' || Number.isNaN(n) || n < 0) {
                throw new RangeError('maxListeners must be a non-negative number');
            }
            this._maxListeners = n;
            return this;
        }
        getMaxListeners() { return this._maxListeners; }
        on(event, listener) {
            if (typeof listener !== 'function') throw new TypeError('listener must be a function');
            let list = this._events.get(event);
            if (!list) { list = []; this._events.set(event, list); }
            list.push({ fn: listener, once: false });
            return this;
        }
        addListener(event, listener) { return this.on(event, listener); }
        prependListener(event, listener) {
            if (typeof listener !== 'function') throw new TypeError('listener must be a function');
            let list = this._events.get(event);
            if (!list) { list = []; this._events.set(event, list); }
            list.unshift({ fn: listener, once: false });
            return this;
        }
        once(event, listener) {
            if (typeof listener !== 'function') throw new TypeError('listener must be a function');
            let list = this._events.get(event);
            if (!list) { list = []; this._events.set(event, list); }
            list.push({ fn: listener, once: true });
            return this;
        }
        prependOnceListener(event, listener) {
            if (typeof listener !== 'function') throw new TypeError('listener must be a function');
            let list = this._events.get(event);
            if (!list) { list = []; this._events.set(event, list); }
            list.unshift({ fn: listener, once: true });
            return this;
        }
        off(event, listener) {
            const list = this._events.get(event);
            if (!list) return this;
            const idx = list.findIndex(item => item.fn === listener);
            if (idx >= 0) list.splice(idx, 1);
            if (!list.length) this._events.delete(event);
            return this;
        }
        removeListener(event, listener) { return this.off(event, listener); }
        removeAllListeners(event) {
            if (event === undefined) this._events.clear();
            else this._events.delete(event);
            return this;
        }
        emit(event, ...args) {
            const list = this._events.get(event);
            if (!list || !list.length) {
                if (event === 'error') {
                    const err = args[0] instanceof Error ? args[0] : new Error(String(args[0] ?? 'Unhandled error'));
                    throw err;
                }
                return false;
            }
            const snapshot = list.slice();
            for (const entry of snapshot) {
                if (entry.once) this.off(event, entry.fn);
                entry.fn.apply(this, args);
            }
            return true;
        }
        listeners(event) { return (this._events.get(event) ?? []).map(entry => entry.fn); }
        listenerCount(event) { return (this._events.get(event) ?? []).length; }
        eventNames() { return Array.from(this._events.keys()); }
    }

    function deepStrictEqualValue(a, b, seen = new Map()) {
        if (Object.is(a, b)) return true;
        if (a === null || b === null || typeof a !== 'object' || typeof b !== 'object') return false;
        if (Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return false;
        if (a instanceof Date) return Object.is(a.getTime(), b.getTime());
        if (a instanceof RegExp) return a.source === b.source && a.flags === b.flags;
        if (ArrayBuffer.isView(a) && ArrayBuffer.isView(b)) {
            if (a.byteLength !== b.byteLength) return false;
            const lhs = new Uint8Array(a.buffer, a.byteOffset, a.byteLength);
            const rhs = new Uint8Array(b.buffer, b.byteOffset, b.byteLength);
            for (let i = 0; i < lhs.length; i++) if (lhs[i] !== rhs[i]) return false;
            return true;
        }
        if (seen.get(a) === b) return true;
        seen.set(a, b);
        if (a instanceof Set) {
            if (a.size !== b.size) return false;
            for (const item of a) {
                if (!b.has(item)) return false;
            }
            return true;
        }
        if (a instanceof Map) {
            if (a.size !== b.size) return false;
            for (const [k, v] of a) {
                if (!b.has(k) || !deepStrictEqualValue(v, b.get(k), seen)) return false;
            }
            return true;
        }
        if (Array.isArray(a)) {
            if (!Array.isArray(b) || a.length !== b.length) return false;
            for (let i = 0; i < a.length; i++) {
                if (!deepStrictEqualValue(a[i], b[i], seen)) return false;
            }
            return true;
        }
        const keysA = Object.keys(a);
        const keysB = Object.keys(b);
        if (keysA.length !== keysB.length) return false;
        for (const k of keysA) {
            if (!Object.prototype.hasOwnProperty.call(b, k)) return false;
            if (!deepStrictEqualValue(a[k], b[k], seen)) return false;
        }
        return true;
    }

    function formatValue(value) {
        if (typeof value === 'string') return value;
        if (value instanceof Error) return value.stack || `${value.name}: ${value.message}`;
        try { return JSON.stringify(value); } catch { return String(value); }
    }

    const util = Object.freeze({
        TextEncoder,
        TextDecoder,
        isDeepStrictEqual: (a, b) => deepStrictEqualValue(a, b),
        inspect(value) { return formatValue(value); },
        format(fmt, ...args) {
            if (typeof fmt !== 'string') return [fmt, ...args].map(formatValue).join(' ');
            let idx = 0;
            let out = fmt.replace(/%[sdifjoO%]/g, token => {
                if (token === '%%') return '%';
                if (idx >= args.length) return token;
                const arg = args[idx++];
                if (token === '%s') return String(arg);
                if (token === '%d' || token === '%f') return String(Number(arg));
                if (token === '%i') return String(parseInt(arg, 10));
                return formatValue(arg);
            });
            while (idx < args.length) out += ' ' + formatValue(args[idx++]);
            return out;
        },
        promisify(fn) {
            if (typeof fn !== 'function') throw new TypeError('promisify requires a function');
            return function (...args) {
                return new Promise((resolve, reject) => {
                    fn.call(this, ...args, (err, value) => err ? reject(err) : resolve(value));
                });
            };
        },
        types: Object.freeze({
            isUint8Array: v => v instanceof Uint8Array,
            isArrayBuffer: v => v instanceof ArrayBuffer,
            isPromise: v => v instanceof Promise,
            isDate: v => v instanceof Date,
            isRegExp: v => v instanceof RegExp,
            isMap: v => v instanceof Map,
            isSet: v => v instanceof Set,
        }),
    });

    class AssertionError extends Error {
        constructor(options = {}) {
            const message = options.message ?? `Assertion failed (${options.operator ?? 'assert'})`;
            super(message);
            this.name = 'AssertionError';
            this.code = 'ERR_ASSERTION';
            this.actual = options.actual;
            this.expected = options.expected;
            this.operator = options.operator;
        }
    }

    function ok(value, message) {
        if (!value) throw new AssertionError({ actual: value, expected: true, operator: '==', message: message instanceof Error ? message.message : (message ?? 'Expected value to be truthy') });
    }
    function strictEqual(actual, expected, message) {
        if (!Object.is(actual, expected)) throw new AssertionError({ actual, expected, operator: 'strictEqual', message });
    }
    function notStrictEqual(actual, expected, message) {
        if (Object.is(actual, expected)) throw new AssertionError({ actual, expected, operator: 'notStrictEqual', message });
    }
    function deepStrictEqual(actual, expected, message) {
        if (!deepStrictEqualValue(actual, expected)) throw new AssertionError({ actual, expected, operator: 'deepStrictEqual', message });
    }
    function notDeepStrictEqual(actual, expected, message) {
        if (deepStrictEqualValue(actual, expected)) throw new AssertionError({ actual, expected, operator: 'notDeepStrictEqual', message });
    }
    function match(value, regexp, message) {
        if (typeof value !== 'string' || !(regexp instanceof RegExp) || !regexp.test(value)) {
            throw new AssertionError({ actual: value, expected: regexp, operator: 'match', message });
        }
    }
    function fail(message) {
        if (message instanceof Error) throw message;
        throw new AssertionError({ operator: 'fail', message: message ?? 'Failed' });
    }
    function matchesExpectedError(err, expected) {
        if (expected === undefined) return true;
        if (expected instanceof RegExp) return expected.test(String(err?.message ?? err));
        if (typeof expected === 'function') return err instanceof expected || Boolean(expected(err));
        if (expected && typeof expected === 'object') {
            return Object.entries(expected).every(([k, v]) => v instanceof RegExp ? v.test(String(err?.[k])) : err?.[k] === v);
        }
        return true;
    }
    function throws(fn, expected, message) {
        let threw = false;
        let caught;
        try { fn(); } catch (err) { threw = true; caught = err; }
        if (!threw) throw new AssertionError({ operator: 'throws', message: message ?? 'Missing expected exception' });
        if (!matchesExpectedError(caught, expected)) throw caught;
    }
    async function rejects(fnOrPromise, expected, message) {
        let threw = false;
        let caught;
        try {
            await (typeof fnOrPromise === 'function' ? fnOrPromise() : fnOrPromise);
        } catch (err) {
            threw = true;
            caught = err;
        }
        if (!threw) throw new AssertionError({ operator: 'rejects', message: message ?? 'Missing expected rejection' });
        if (!matchesExpectedError(caught, expected)) throw caught;
    }

    const assertApi = Object.assign(ok, {
        ok,
        equal: strictEqual,
        notEqual: notStrictEqual,
        strictEqual,
        notStrictEqual,
        deepEqual: deepStrictEqual,
        deepStrictEqual,
        notDeepStrictEqual,
        throws,
        rejects,
        match,
        fail,
        AssertionError,
    });
    assertApi.strict = assertApi;
    Object.freeze(assertApi);

    const querystring = Object.freeze({
        escape: str => encodeURIComponent(String(str)),
        unescape: str => decodeURIComponent(String(str).replace(/\+/g, ' ')),
        parse(str, sep = '&', eq = '=') {
            const out = Object.create(null);
            if (typeof str !== 'string' || !str.length) return out;
            for (const part of str.split(sep)) {
                if (!part) continue;
                const idx = part.indexOf(eq);
                const k = querystring.unescape(idx >= 0 ? part.slice(0, idx) : part);
                const v = querystring.unescape(idx >= 0 ? part.slice(idx + eq.length) : '');
                if (Object.prototype.hasOwnProperty.call(out, k)) {
                    if (Array.isArray(out[k])) out[k].push(v);
                    else out[k] = [out[k], v];
                } else {
                    out[k] = v;
                }
            }
            return out;
        },
        stringify(obj, sep = '&', eq = '=') {
            if (!obj || typeof obj !== 'object') return '';
            const parts = [];
            for (const [k, v] of Object.entries(obj)) {
                const key = querystring.escape(k);
                if (Array.isArray(v)) {
                    for (const item of v) parts.push(`${key}${eq}${querystring.escape(item ?? '')}`);
                } else if (v !== undefined) {
                    parts.push(`${key}${eq}${querystring.escape(v ?? '')}`);
                }
            }
            return parts.join(sep);
        },
    });

    const nodeOs = Object.freeze({
        EOL: '\n',
        platform: () => 'linux',
        arch: () => 'x64',
        endianness: () => 'LE',
        constants: Object.freeze({ errno: Object.freeze({}), signals: Object.freeze({}) }),
    });

    function structuredCloneImpl(value, seen = new WeakMap()) {
        if (value === null || typeof value !== 'object') {
            if (typeof value === 'function' || typeof value === 'symbol') {
                throw new TypeError('Value cannot be cloned');
            }
            return value;
        }
        if (seen.has(value)) return seen.get(value);
        if (value instanceof Date) return new Date(value.getTime());
        if (value instanceof RegExp) return new RegExp(value.source, value.flags);
        if (value instanceof Buffer) return Buffer.from(value);
        if (ArrayBuffer.isView(value)) {
            return new value.constructor(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength));
        }
        if (value instanceof ArrayBuffer) return value.slice(0);
        if (value instanceof Map) {
            const out = new Map();
            seen.set(value, out);
            for (const [k, v] of value) out.set(structuredCloneImpl(k, seen), structuredCloneImpl(v, seen));
            return out;
        }
        if (value instanceof Set) {
            const out = new Set();
            seen.set(value, out);
            for (const item of value) out.add(structuredCloneImpl(item, seen));
            return out;
        }
        if (Array.isArray(value)) {
            const out = [];
            seen.set(value, out);
            for (let i = 0; i < value.length; i++) out[i] = structuredCloneImpl(value[i], seen);
            return out;
        }
        const out = {};
        seen.set(value, out);
        for (const [k, v] of Object.entries(value)) out[k] = structuredCloneImpl(v, seen);
        return out;
    }

    globalThis.TextEncoder = TextEncoder;
    globalThis.TextDecoder = TextDecoder;
    globalThis.Buffer = Buffer;
    globalThis.URLSearchParams = URLSearchParams;
    globalThis.btoa = str => encodeBase64(encodeLatin1(str), false);
    globalThis.atob = b64 => decodeLatin1(decodeBase64(b64));
    globalThis.structuredClone = value => structuredCloneImpl(value);
    globalThis.agenaCordisNodePath = posix;
    globalThis.agenaCordisNodeEvents = EventEmitter;
    globalThis.agenaCordisNodeUtil = util;
    globalThis.agenaCordisNodeAssert = assertApi;
    globalThis.agenaCordisNodeQuerystring = querystring;
    globalThis.agenaCordisNodeOs = nodeOs;
})();
