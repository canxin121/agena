((manager, initial, config) => {
    const units = new Map();
    const closedBackends = new Set();
    let snapshots = initial ?? {};
    const namePattern = /^[a-z][a-z0-9_]{0,63}$/;
    const recordKey = /^[a-zA-Z0-9_-]+$/;
    const fault = (code, message) => Object.assign(new Error(message), { name: 'StorageError', code });
    const own = (object, key) => Object.hasOwn(object, key);
    const json = value => JSON.stringify(value, (_, item) => {
        if (item === undefined || typeof item === 'function' || typeof item === 'symbol'
            || typeof item === 'number' && !Number.isFinite(item)) {
            throw fault('malformed-medium', 'Cordis storage values must be JSON data');
        }
        return item;
    });
    const copy = value => JSON.parse(json(value));
    const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
    const collection = kind => {
        const backend = snapshots[kind] ??= {};
        if (backend.failure) throw fault('malformed-medium', backend.failure);
        return backend.units ??= {};
    };
    const lookup = (kind, name) => {
        const entries = collection(kind);
        return own(entries, name) ? entries[name] : null;
    };

    function openUnit(descriptor, backend, decode) {
        if (closedBackends.has(backend)) throw fault('closed', 'Cordis storage backend ' + backend + ' is closed');
        const { name, version, tables, hasGlobal, layout, compatibleVersions = [] } = descriptor;
        const validName = name => typeof name === 'string' && namePattern.test(name);
        if (!validName(name) || !Number.isSafeInteger(version) || version < 0
            || !Array.isArray(tables) || tables.length > 64 || tables.some(name => !validName(name))
            || new Set(tables).size !== tables.length || typeof hasGlobal !== 'boolean'
            || !Array.isArray(compatibleVersions)
            || compatibleVersions.some(value => !Number.isSafeInteger(value) || value < 0 || value >= version)) {
            throw fault('malformed-medium', 'Invalid Cordis storage unit descriptor');
        }
        if (layout !== undefined && layout !== 'single' && layout !== 'per-record') {
            throw fault('unsupported-layout', 'Unknown Cordis storage layout ' + String(layout));
        }
        const kind = backend === 'sqlite' ? 'sqlite' : layout === 'per-record' ? 'json_records' : 'single';
        const unitId = backend + ':' + name;
        if (units.has(unitId) || units.size >= 64) {
            throw fault('already-open', 'Cordis storage unit ' + name + ' cannot be opened again');
        }
        const accepts = stamp => stamp === version || compatibleVersions.includes(stamp);
        const empty = () => ({
            version, revision: 0,
            tables: Object.fromEntries(tables.map(table => [table, {}])),
            global: null,
        });
        let saved = null;
        let rawState;
        let state;
        let closing;
        let closed = false;
        let queue = Promise.resolve();
        let seed = [];
        let invalid = [];
        const assertReadable = () => {
            if (closed) throw fault('closed', 'Cordis storage unit ' + name + ' is closed');
        };
        const assertTable = table => {
            assertReadable();
            if (!tables.includes(table)) throw fault('unknown-table', 'Unknown Cordis table ' + String(table));
        };
        const assertKey = key => {
            if (typeof key !== 'string') throw new TypeError('Cordis storage keys must be strings');
            if (kind === 'json_records' && !recordKey.test(key)) {
                throw fault('invalid-key', 'Cordis per-record JSON keys must be path-safe');
            }
        };
        const assign = (value, table, key, record) => {
            Object.defineProperty(value.tables[table], key, {
                value: record, configurable: true, writable: true, enumerable: true,
            });
        };
        const checkVersion = stamp => {
            if (!accepts(stamp)) {
                throw fault('version-mismatch', 'Cordis unit ' + name + ' has version ' + stamp + '; expected ' + version);
            }
        };
        const parseSingle = raw => {
            let next;
            try { next = JSON.parse(raw); } catch { throw fault('malformed-medium', 'Invalid saved Cordis unit ' + name); }
            if (!object(next) || !object(next.tables) || !Number.isSafeInteger(next.revision) || next.revision < 1
                || !own(next, 'global') || tables.some(table => own(next.tables, table) && !object(next.tables[table]))) {
                throw fault('malformed-medium', 'Invalid saved Cordis unit ' + name);
            }
            checkVersion(next.version);
            for (const table of tables) {
                if (!own(next.tables, table)) next.tables[table] = {};
            }
            return next;
        };
        const read = () => {
            const raw = lookup(kind, name);
            let next = empty();
            seed = [];
            invalid = [];
            if (kind === 'single') {
                if (raw !== null) next = parseSingle(raw);
                saved = raw;
            } else {
                saved = raw?.revision ?? null;
                if (raw) {
                    if (kind === 'sqlite') checkVersion(raw.version);
                    for (const {table, key, version: stamp, value} of raw.entries) {
                        if (kind === 'json_records' && !accepts(stamp)) continue;
                        if (table === '' && hasGlobal) next.global = value;
                        else if (tables.includes(table)) assign(next, table, key, value);
                    }
                } else if (kind === 'json_records') {
                    const legacy = lookup('single', name);
                    if (legacy !== null) {
                        try { next = parseSingle(legacy); } catch (error) {
                            if (!['malformed-medium', 'version-mismatch'].includes(error.code)) throw error;
                        }
                        for (const table of tables) {
                            for (const [key, value] of Object.entries(next.tables[table])) {
                                if (!recordKey.test(key)) continue;
                                seed.push({ operation: 'put', table, key, value });
                            }
                        }
                        if (hasGlobal && next.global !== null) {
                            seed.push({ operation: 'put', table: '', key: '', value: next.global });
                        }
                        next = empty();
                        for (const {table, key, value} of seed) {
                            if (table) assign(next, table, key, value);
                            else next.global = value;
                        }
                    }
                }
            }
            rawState = next;
            state = decode ? decode(copy(next), invalid) : next;
        };
        read();

        const transaction = edits => ({ backend: kind, unit: name, expected: saved, version, edits: [...seed, ...edits] });
        const rowIdentity = ({table, key}) => JSON.stringify([table, key]);
        const confirm = (committed, edits) => {
            const entries = new Map((lookup(kind, name)?.entries ?? []).map(row => [rowIdentity(row), row]));
            for (const edit of edits) {
                const id = rowIdentity(edit);
                if (edit.operation === 'put') {
                    entries.set(id, {table: edit.table, key: edit.key, version, value: edit.value});
                } else {
                    entries.delete(id);
                }
            }
            saved = committed.revision;
            collection(kind)[name] = {revision: saved, version, entries: [...entries.values()]};
            seed = [];
            invalid = [];
        };
        const backupEdits = () => invalid.map(({table, key}) => ({ operation: 'backup', table, key }));
        const startup = () => seed.length || invalid.length ? transaction(backupEdits()) : null;
        const change = (action, prepare = false) => {
            if (closing) return Promise.reject(fault('closed', 'Cordis storage unit ' + name + ' is closing'));
            const run = queue.then(async () => {
                assertReadable();
                const draft = copy(rawState);
                const events = [];
                const value = action(draft, events);
                if (!events.length && !seed.length) return { value };
                draft.version = version;
                draft.revision++;
                const decoded = decode ? decode(copy(draft), null) : draft;
                let backups = [];
                if (kind === 'single') {
                    const raw = json(draft);
                    if (!await manager.host('storage', { unit: name, previous: saved, value: raw })) {
                        throw fault('conflict', 'Cordis storage unit ' + name + ' changed in another invocation');
                    }
                    saved = raw;
                    collection(kind)[name] = raw;
                } else {
                    const request = transaction(events.map(({operation, table, key, value}) => ({
                        operation: operation === 'deleted' ? 'delete' : operation, table, key,
                        ...(operation === 'put' && {value}),
                    })));
                    const committed = await manager.host('storage_records', request);
                    if (!committed) throw fault('conflict', 'Cordis storage unit ' + name + ' changed in another invocation');
                    confirm(committed, request.edits);
                    backups = committed.backups;
                }
                rawState = draft;
                state = decoded;
                if (decode && !prepare) {
                    for (const event of events) {
                        if (event.operation === 'put') {
                            event.value = event.table ? state.tables[event.table][event.key] : state.global;
                        }
                        manager.emit('domain/changed', copy(event));
                    }
                }
                return { value, backups };
            });
            queue = run.catch(() => {});
            return run;
        };
        const prepare = () => {
            if (!seed.length && !invalid.length) return;
            return change((draft, events) => {
                for (const edit of backupEdits()) {
                    delete draft.tables[edit.table][edit.key];
                    events.push(edit);
                }
            }, true);
        };
        const upsert = (draft, events, table, key, value) => {
            const record = copy(value);
            if (table) assign(draft, table, key, record);
            else draft.global = record;
            events.push({ domain: name, table, key, operation: 'put', value: record });
        };
        const remove = (draft, events, table, keys, includeForeign = false) => {
            let removed = 0;
            const physical = includeForeign && kind === 'json_records'
                ? new Set((lookup(kind, name)?.entries ?? []).filter(row => row.table === table).map(row => row.key))
                : null;
            for (const key of keys) {
                if (own(draft.tables[table], key)) {
                    delete draft.tables[table][key];
                    removed++;
                } else if (!physical?.has(key)) continue;
                events.push({ domain: name, table, key, operation: 'deleted' });
            }
            return removed;
        };
        const close = () => closing ??= queue.then(() => {
            closed = true;
            units.delete(unitId);
        });
        units.set(unitId, { read, close, startup, prepare, backend });
        const document = () => {
            assertReadable();
            return state;
        };
        const setGlobal = async value => {
            if (!hasGlobal) throw fault('invalid-record', 'Cordis storage requires a declared global');
            await change((draft, events) => upsert(draft, events, '', '', value));
        };
        const unit = Object.freeze({
            async loadAll() {
                const current = document();
                return copy({ tables: Object.fromEntries(tables.map(table => [table, current.tables[table]])), global: hasGlobal ? current.global : null });
            },
            async putRecord(table, key, value) {
                assertTable(table); assertKey(key);
                await change((draft, events) => upsert(draft, events, table, key, value));
            },
            async deleteRecord(table, key) {
                await unit.deleteRecords(table, [key]);
            },
            async deleteRecords(table, keys) {
                assertTable(table);
                if (!Array.isArray(keys)) throw new TypeError('Cordis deleteRecords requires keys');
                const requested = [...new Set(keys)];
                requested.forEach(assertKey);
                await change((draft, events) => remove(draft, events, table, requested, !decode));
            },
            ...(kind === 'json_records' && {
                async backupRecord(table, key) {
                    assertTable(table); assertKey(key);
                    const {backups} = await change((draft, events) => {
                        delete draft.tables[table][key];
                        events.push({operation: 'backup', table, key});
                    }, true);
                    return backups[0];
                },
            }),
            setGlobal,
            putGlobal: setGlobal,
            close,
        });
        return { unit, document, change, upsert, remove, assertTable, assertKey, prepare };
    }

    const domains = new Map();
    const reserved = new Set();
    const backends = new Map(config.backends.map(name => {
        let closing;
        return [name, Object.freeze({
            kv: Object.freeze({ async open(descriptor) {
                const connection = openUnit(descriptor, name);
                try {
                    if (manager.active) await connection.prepare();
                    return connection.unit;
                } catch (error) {
                    await connection.unit.close();
                    throw error;
                }
            } }),
            close() {
                closedBackends.add(name);
                return closing ??= Promise.all([...units.values()].filter(unit => unit.backend === name)
                    .map(unit => unit.close())).then(() => {});
            },
        })];
    }));
    const backendFor = name => {
        const backend = backends.get(name);
        if (!backend) throw fault('backend-not-found', 'Cordis storage backend ' + String(name) + ' is unavailable');
        return backend;
    };

    const openDomain = async spec => {
        if (!object(spec?.tables) || spec.invalidRecords !== undefined && spec.invalidRecords !== 'backup-and-skip') {
            throw fault('invalid-spec', 'Invalid Cordis domain declaration');
        }
        if (reserved.has(spec.name)) throw fault('already-open', 'Cordis domain ' + spec.name + ' is already open');
        const backend = own(config.routes, spec.name) ? config.routes[spec.name] : config.defaultBackend;
        backendFor(backend);
        const names = Object.keys(spec.tables);
        const hasGlobal = spec.global !== undefined;
        const recoverRecords = backend === 'json' && spec.layout === 'per-record'
            && spec.invalidRecords === 'backup-and-skip';
        const parse = (schema, value) => {
            try { return copy(schema.parse(copy(value))); }
            catch { throw fault('invalid-record', 'Stored Cordis domain ' + spec.name + ' does not match its schema'); }
        };
        const initialGlobal = hasGlobal ? parse(spec.global.schema, spec.global.initial) : null;
        if (hasGlobal && (initialGlobal === null || spec.global.schema.safeParse?.(null)?.success === true)) {
            throw fault('invalid-spec', 'Cordis domain globals cannot be nullable');
        }
        const connection = openUnit({ ...spec, tables: names, hasGlobal }, backend, (state, bad) => {
            for (const table of names) {
                for (const [key, value] of Object.entries(state.tables[table])) {
                    try {
                        state.tables[table][key] = parse(spec.tables[table].valueSchema, value);
                    } catch (error) {
                        if (!recoverRecords || bad === null) throw error;
                        bad.push({table, key});
                        delete state.tables[table][key];
                    }
                }
            }
            if (state.global !== null && hasGlobal) state.global = parse(spec.global.schema, state.global);
            return state;
        });
        reserved.add(spec.name);
        const { unit, document, change, upsert, remove, assertTable, assertKey } = connection;
        try {
            if (manager.active) await connection.prepare();
            const tables = new Map(names.map(table => {
                const records = () => document().tables[table];
                return [table, Object.freeze({
                    get size() { return Object.keys(records()).length; },
                    get(key) { return own(records(), key) ? copy(records()[key]) : undefined; },
                    entries() { return Object.entries(copy(records()))[Symbol.iterator](); },
                    keys() { return Object.keys(records())[Symbol.iterator](); },
                    put: (key, value) => unit.putRecord(table, key, value),
                    delete(key) { return this.deleteMany([key]).then(count => count > 0); },
                    async deleteMany(keys) {
                        const requested = [...new Set(keys)];
                        requested.forEach(assertKey);
                        const {value} = await change((draft, events) => remove(draft, events, table, requested));
                        return value;
                    },
                    async update(key, update) {
                        assertKey(key);
                        if (typeof update !== 'function') throw new TypeError('Cordis table updates require a function');
                        const {value} = await change((draft, events) => {
                            const current = records();
                            if (!own(current, key)) throw fault('missing-key', 'Missing key in Cordis table ' + table);
                            const next = copy(update(copy(current[key])));
                            upsert(draft, events, table, key, next);
                            return next;
                        });
                        return value;
                    },
                })];
            }));
            const domain = Object.freeze({
                name: spec.name,
                table(name) { assertTable(name); return tables.get(name); },
                ...(hasGlobal && { global: Object.freeze({
                    get: () => copy(document().global ?? initialGlobal),
                    set: value => {
                        if (value === null) throw fault('invalid-record', 'Cordis domain globals cannot be nullable');
                        return unit.setGlobal(value);
                    },
                }) }),
                async close() {
                    await unit.close();
                    if (domains.get(spec.name) === domain) {
                        domains.delete(spec.name);
                        reserved.delete(spec.name);
                    }
                },
            });
            domains.set(spec.name, domain);
            return domain;
        } catch (error) {
            await unit.close();
            reserved.delete(spec.name);
            throw error;
        }
    };

    const domain = Object.freeze({
        open: openDomain,
        get: name => domains.get(name),
        async closeAll() { await Promise.all([...domains.values()].map(domain => domain.close())); },
    });
    const service = Object.freeze({
        domain,
        form(name) {
            if (name !== 'domain') throw fault('form-not-mounted', 'Cordis storage form ' + String(name) + ' is unavailable');
            return domain;
        },
        backend: Object.freeze({ get: backendFor, names: () => [...backends.keys()] }),
    });
    return {
        service, domain, backends: Object.freeze([...backends.keys()]),
        startup: () => [...units.values()].map(unit => unit.startup()).filter(request => request !== null),
        async prepare() {
            for (const unit of units.values()) await unit.prepare();
        },
        refresh(next) {
            snapshots = next ?? {};
            for (const unit of units.values()) unit.read();
        },
    };
})
