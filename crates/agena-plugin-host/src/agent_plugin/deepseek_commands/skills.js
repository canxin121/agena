((manager) => {
    const providers = new Map();
    const runtime = new Map();
    const both = Object.freeze({modelInvocable: true, userInvocable: true});
    const namePattern = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
    const changed = () => manager.emit('skills/change');
    const invocation = value => {
        if (value === undefined) return both;
        if (typeof value?.modelInvocable !== 'boolean' || typeof value?.userInvocable !== 'boolean') {
            throw new TypeError('invalid Cordis skill invocation policy');
        }
        return {modelInvocable: value.modelInvocable, userInvocable: value.userInvocable};
    };
    const projectBase = base => {
        if (base === undefined) return undefined;
        if (base?.kind === 'directory' && typeof base.path === 'string') return {kind: base.kind, path: base.path};
        if (base?.kind === 'url' && typeof base.url === 'string') return {kind: base.kind, url: base.url};
        if (base?.kind === 'opaque' && typeof base.description === 'string') return {kind: base.kind, description: base.description};
        throw new TypeError('invalid Cordis skill resource base');
    };
    const summary = skill => {
        const {name, description, path, whenToUse, provider, source, resourceBase} = skill;
        if (typeof name !== 'string' || name.length > 64 || !namePattern.test(name)
            || typeof description !== 'string' || !description.trim() || description.length > 4096) {
            throw new TypeError('invalid Cordis skill name or description');
        }
        return {name, description, path, whenToUse, provider, source, resourceBase: projectBase(resourceBase),
            invocation: invocation(skill.invocation)};
    };
    const definition = skill => {
        const projected = summary(skill);
        if (typeof skill.content !== 'string' || !skill.content.trim() || skill.content.length > 1048576) {
            throw new TypeError(`invalid Cordis skill content for ${projected.name}`);
        }
        return {...projected, content: skill.content, metadata: skill.metadata};
    };

    const registerProvider = create => {
        if (typeof create !== 'function' || providers.size >= 64) {
            throw new TypeError('invalid Cordis skill provider registration');
        }
        const controller = new AbortController();
        let active = false;
        try {
            const provider = create(Object.freeze({
                signal: controller.signal,
                invalidate() { if (active) changed(); },
            }));
            const name = provider?.name;
            if (typeof name !== 'string' || !name.trim() || name.length > 128 || name === 'runtime'
                || typeof provider.list !== 'function' || typeof provider.get !== 'function' || providers.has(name)) {
                throw new TypeError('invalid or duplicate Cordis skill provider');
            }
            providers.set(name, provider);
            active = true;
            changed();
            return () => {
                if (!active) return;
                active = false;
                providers.delete(name);
                controller.abort(new Error('Cordis skill provider disposed'));
                changed();
            };
        } catch (error) {
            controller.abort(error);
            throw error;
        }
    };

    const register = skill => {
        const registered = definition({...skill, provider: skill.provider ?? 'runtime', source: skill.source ?? 'runtime'});
        if (runtime.has(registered.name)) return () => {};
        if (runtime.size >= 128) throw new Error('too many Cordis runtime skills');
        runtime.set(registered.name, registered);
        changed();
        return () => {
            if (runtime.get(registered.name) !== registered) return;
            runtime.delete(registered.name);
            changed();
        };
    };

    const collect = async (options = {}) => {
        const candidates = new Map();
        const diagnostics = [];
        const add = (candidate, rank, load) => {
            const current = candidates.get(candidate.name);
            if (current === undefined && candidates.size >= 128) throw new Error('too many Cordis skill candidates');
            if (current === undefined || rank < current.rank) {
                candidates.set(candidate.name, {candidate, rank, load});
            }
        };
        let complete = true;
        for (const skill of runtime.values()) add(summary(skill), 250, () => skill);
        const lookup = Object.freeze({cwd: options.cwd, signal: options.signal});
        for (const [name, provider] of providers) {
            options.signal?.throwIfAborted();
            try {
                const observation = await provider.list(lookup);
                const list = Array.isArray(observation) ? observation : observation?.candidates;
                if (!Array.isArray(list) || list.length > 128) throw new TypeError('invalid skill candidate list');
                complete &&= Array.isArray(observation) || observation.complete === true;
                for (const candidate of list) {
                    if (!Number.isFinite(candidate?.rank)) continue;
                    add(summary(candidate), candidate.rank, () => provider.get(candidate, lookup));
                }
            } catch (error) {
                complete = false;
                diagnostics.push(`${name}: ${error?.message ?? String(error)}`.slice(0, 1024));
            }
        }
        options.signal?.throwIfAborted();
        return {candidates, diagnostics, complete};
    };

    const load = async entry => {
        const value = await entry.load();
        if (value === undefined) return undefined;
        const skill = definition(value);
        if (skill.name !== entry.candidate.name) throw new Error('Cordis skill changed its name during load');
        skill.resourceBase ??= entry.candidate.resourceBase;
        skill.invocation = {
            modelInvocable: skill.invocation.modelInvocable && entry.candidate.invocation.modelInvocable,
            userInvocable: skill.invocation.userInvocable && entry.candidate.invocation.userInvocable,
        };
        return skill;
    };

    const listSnapshot = async options => {
        const {candidates, complete} = await collect(options);
        const skills = Array.from(candidates.values(), entry => entry.candidate);
        skills.sort((left, right) => left.name.localeCompare(right.name));
        return {skills, complete};
    };

    return {
        service: Object.freeze({
            registerProvider,
            register,
            listSnapshot,
            async list(options) { return (await listSnapshot(options)).skills; },
            async get(name, options) {
                const entry = (await collect(options)).candidates.get(name);
                return entry === undefined ? undefined : await load(entry);
            },
        }),
        async snapshot(options) {
            const {candidates, diagnostics} = await collect(options);
            const skills = [];
            let remaining = 1048576;
            for (const entry of candidates.values()) {
                try {
                    const skill = await load(entry);
                    if (skill === undefined) continue;
                    const {name, description, content, invocation, resourceBase} = skill;
                    const projected = {name, description, content, invocation, resourceBase, rank: entry.rank};
                    remaining -= JSON.stringify(projected).length;
                    if (remaining < 0) {
                        diagnostics.push('Cordis skill snapshot exceeds its size limit');
                        break;
                    }
                    skills.push(projected);
                } catch (error) {
                    diagnostics.push(`${entry.candidate.name}: ${error?.message ?? String(error)}`.slice(0, 1024));
                }
            }
            return {skills, diagnostics};
        },
    };
})
