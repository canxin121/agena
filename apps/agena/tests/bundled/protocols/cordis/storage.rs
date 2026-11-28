use agena_plugin_sdk::CommandStatus;
use agena_testkit::{TestCase, TestResult, UserSession};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

const PLUGIN: &str = "deepseek.fixture-storage";

async fn install(case: TestCase) -> TestResult {
    let root = case.paths().root.join("cordis-storage");
    tokio::fs::create_dir_all(&root).await?;
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    tokio::fs::write(
        root.join("package.json"),
        br#"{"name":"@fixture/storage","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
    )
    .await?;
    tokio::fs::write(
        root.join("bundle.yml"),
        r#"
- insert:
    - name: '@deepseek-ai/dsh-storage'
    - name: '@deepseek-ai/dsh-storage-json'
      config:
        root: !!js dshHomePath('fixture-storage')
    - name: '@deepseek-ai/dsh-storage-sqlite'
      config:
        path: !!js dshHomePath('fixture.sqlite')
        journalMode: delete
    - name: '@deepseek-ai/dsh-storage-domain'
      config:
        backend: json
        routes: {sql: sqlite}
    - name: ./store.mjs
    - name: ./cache.mjs
"#,
    )
    .await?;
    tokio::fs::write(root.join("store.mjs"), r#"
import { domainTable } from '@deepseek-ai/dsh-storage-domain';
export const inject = ['commands', 'storage', 'storageDomain', 'storage.backend.sqlite'];
const integer = {parse(value) {
    if (!Number.isSafeInteger(value)) throw new Error('expected integer');
    return value;
}};
const descriptor = (name, version, layout, compatibleVersions = []) =>
    ({name, version, layout, compatibleVersions, tables: ['rows'], hasGlobal: true});
const sqlSpec = (version, compatibleVersions = []) =>
    ({name: 'sql', version, compatibleVersions, layout: 'per-record', tables: {rows: domainTable(integer)}});
export function apply(ctx) {
    const json = ctx.storage.backend.get('json');
    const sqlite = ctx.get('storage.backend.sqlite');
    if (sqlite !== ctx.storage.backend.get('sqlite')) throw new Error('wrong SQLite service');
    ctx.commands.register({name: 'store', description: 'Test Cordis storage', async handler({rawInput}) {
        const result = {};
        if (rawInput === 'seed') {
            const legacy = await json.kv.open(descriptor('legacy', 1, 'single'));
            await legacy.putRecord('rows', 'good', 33);
            await legacy.setGlobal(34);
            await legacy.close();
            const old = await json.kv.open(descriptor('cache', 1, 'per-record'));
            await old.putRecord('rows', 'stale', 10);
            await old.close();
            const current = await json.kv.open(descriptor('cache', 2, 'per-record'));
            await current.putRecord('rows', 'good', 20);
            await current.putRecord('rows', 'bad', 'keep the original bytes');
            await current.putRecord('rows', 'temporary', 25);
            try { await current.deleteRecords('rows', ['temporary', '../unsafe']); }
            catch (error) { result.unsafeBatch = error.code; }
            result.backup = await current.backupRecord('rows', 'temporary');
            try { await current.putRecord('rows', '../unsafe', 40); }
            catch (error) { result.unsafe = error.code; }
            result.versions = await current.loadAll();
            await current.close();
            const sql = await ctx.storageDomain.open(sqlSpec(1));
            await sql.table('rows').put('opaque / key', 50);
            await sql.table('rows').put('untouched', 60);
            await sql.close();
        } else {
            const legacy = await json.kv.open(descriptor('legacy', 2, 'per-record', [1]));
            result.legacy = await legacy.loadAll();
            await legacy.close();
            const cache = await json.kv.open(descriptor('cache', 2, 'per-record', [1]));
            result.cache = await cache.loadAll();
            await cache.close();
            if (rawInput === 'upgrade') {
                try { await ctx.storageDomain.open(sqlSpec(2)); }
                catch (error) { result.mismatch = error.code; }
            }
            const sql = await ctx.storageDomain.open(sqlSpec(2, [1]));
            if (rawInput === 'upgrade') await sql.table('rows').update('opaque / key', value => value + 1);
            result.sql = Object.fromEntries(sql.table('rows').entries());
            await sql.close();
            if (rawInput === 'upgrade') {
                try { await ctx.storageDomain.open(sqlSpec(1)); }
                catch (error) { result.old = error.code; }
            }
        }
        return {kind: 'success', text: JSON.stringify(result)};
    }});
}
"#).await?;
    tokio::fs::write(root.join("cache.mjs"), r#"
import { domainTable } from '@deepseek-ai/dsh-storage-domain';
export const inject = ['commands', 'storageDomain'];
export async function apply(ctx) {
    const view = await ctx.storageDomain.open({
        name: 'cache', version: 2, layout: 'per-record', invalidRecords: 'backup-and-skip',
        tables: {rows: domainTable({parse(value) {
            if (!Number.isSafeInteger(value)) throw new Error('expected integer');
            return value;
        }})},
    });
    ctx.commands.register({name: 'cache_view', description: 'Read current cache rows', handler() {
        return {kind: 'success', text: JSON.stringify(Object.fromEntries(view.table('rows').entries()))};
    }});
}
"#).await?;
    case.run_cli([
        "plugin",
        "install",
        root.to_str().context("Cordis package path")?,
    ])
    .await?
    .require_success()?;
    Ok(())
}

async fn command(user: &UserSession, command: &str, input: &str) -> anyhow::Result<Value> {
    let result = user
        .client()
        .invoke_plugin_command(PLUGIN, command, json!({}), Some(user.id()), None, input)
        .await?
        .result;
    ensure!(result.status == CommandStatus::Succeeded, "{result:?}");
    Ok(serde_json::from_str(&result.summary)?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_cordis_routes_sqlite_and_versioned_records() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            let seed = command(&user, "store", "seed").await?;
            ensure!(seed["unsafe"] == "invalid-key", "{seed}");
            ensure!(seed["unsafeBatch"] == "invalid-key", "{seed}");
            ensure!(
                seed["backup"]
                    .as_str()
                    .is_some_and(|location| location.contains(".sqlite3#cordis_backups/")),
                "{seed}"
            );
            ensure!(
                seed["versions"]["tables"]["rows"]
                    == json!({"bad":"keep the original bytes","good":20}),
                "{seed}"
            );

            let recovered = command(&user, "cache_view", "").await?;
            ensure!(recovered == json!({"good":20}), "{recovered}");
            let upgraded = command(&user, "store", "upgrade").await?;
            ensure!(
                upgraded["legacy"] == json!({"tables":{"rows":{"good":33}},"global":34}),
                "{upgraded}"
            );
            ensure!(
                upgraded["cache"]["tables"]["rows"] == json!({"good":20,"stale":10}),
                "{upgraded}"
            );
            ensure!(
                upgraded["mismatch"] == "version-mismatch" && upgraded["old"] == "version-mismatch",
                "{upgraded}"
            );
            ensure!(
                upgraded["sql"] == json!({"opaque / key":51,"untouched":60}),
                "{upgraded}"
            );

            case.restart().await?;
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().attach_session(user.id()).await?;
            let saved = command(&user, "store", "read").await?;
            for key in ["legacy", "cache", "sql"] {
                ensure!(saved[key] == upgraded[key], "{saved}");
            }
            Ok(())
        })
        .await
}
