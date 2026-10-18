#!/usr/bin/env python3
"""Run the current cutover matrix serially and retain every child result.

Build the release examples first. Full runs require no competing Cargo build
or resource probe. Smoke runs are only behavior checks, never timing evidence.
"""

import argparse
import datetime
import hashlib
import itertools
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import time


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def output(command, root):
    return subprocess.check_output(command, cwd=root, text=True).strip()


def save_process_activity(root, path):
    # ps comm contains no arguments. Persist only its basename so evidence
    # does not disclose unrelated application paths or command lines.
    processes = []
    for line in output(['ps', '-axo', 'pid,ppid,%cpu,%mem,comm'], root).splitlines()[1:]:
        pid, parent, cpu, memory, command = line.split(None, 4)
        if float(cpu) >= 1.0:
            processes.append({
                'pid': int(pid), 'ppid': int(parent),
                'cpu_percent': float(cpu), 'memory_percent': float(memory),
                'comm': Path(command).name,
            })
    processes.sort(key=lambda process: (-process['cpu_percent'], process['pid']))
    path.write_text(json.dumps({
        'sampled_utc': now(), 'minimum_reported_cpu_percent': 1.0,
        'load_average': list(os.getloadavg()), 'processes': processes,
    }, indent=2) + '\n')


def cases(mode):
    if mode == 'smoke':
        return [
            ('b01-small', 'kernel_content_probe', ['1', '1', 'small', 'text']),
            ('b01-large', 'kernel_content_probe', ['1', '1', 'large', 'text']),
            ('b02-log-small', 'kernel_content_probe', ['1', '1', 'small', 'log']),
            ('b02-log-large', 'kernel_content_probe', ['1', '1', 'large', 'log']),
            ('b03', 'kernel_history_probe', ['1000', '0']),
            ('b05', 'kernel_lifecycle_probe', ['b05', '3,10']),
            ('b06', 'kernel_lifecycle_probe', ['b06', '1000', '4', '4']),
            ('b07', 'kernel_lifecycle_probe', ['b07', '1', '1', '32', '32']),
            ('b08', 'kernel_export_probe', ['10', '1', '1', '0', '2']),
            ('b02-api', 'content_delivery_probe', []),
        ]
    selected = [
        (f"{'b01-text' if kind == 'text' else 'b02-log'}-s{sources}-o{observers}-{slicing}-r{repeat}", 'kernel_content_probe',
         [str(sources), str(observers), slicing, kind])
        for repeat, kind, sources, observers, slicing in itertools.product(
            [1, 2, 3], ['text', 'log'], [1, 8, 32], [1, 8], ['small', 'large'])
    ]
    selected += [
        (f'b03-e{entries}' if cold == 0 else f'b04-c{cold}', 'kernel_history_probe',
         [str(entries), str(cold)])
        for entries, cold in [(1000, 0), (10000, 0), (100000, 0), (1000, 64), (1000, 1024)]
    ]
    selected += [
        (f'{case}-r{repeat}', 'kernel_lifecycle_probe', [case])
        for case in ['b05', 'b06', 'b07'] for repeat in [1, 2, 3]
    ]
    selected += [
        (f'b08-{name}-r{repeat}', 'kernel_export_probe', [str(rows), str(text), str(blobs), '1', '2'])
        for name, rows, text, blobs in [('small', 100, 8, 8), ('large', 1000, 128, 128)]
        for repeat in [1, 2, 3]
    ]
    selected += [(f'b02-api-r{repeat}', 'content_delivery_probe', []) for repeat in [1, 2, 3]]
    return selected


def validate(binary, rows, full):
    assert rows, 'probe emitted no JSON records'
    if binary == 'kernel_content_probe':
        assert len(rows) == 1
        row = rows[0]
        assert row['all_observers_exact']
        assert row['stored_body_bytes'] == row['expected_stored_body_bytes']
        if row['kind'] == 'log':
            assert row['all_log_order_channels_timestamps_exact']
        assert row['command_results_during_content'] == row['command_transactions_during_content'] == 1
    elif binary == 'kernel_history_probe':
        assert len(rows) == 1
        row = rows[0]
        assert row['fixture']['cold_body_bytes'] >= row['cold_mib'] * 1024**2
        assert len(row['runs']) == 3
        for sample in row['runs']:
            assert len(sample['reopens']) == 5
            for reopen in sample['reopens']:
                assert reopen['warm_reads'] == 512
                assert reopen['warm_queries'] == reopen['warm_entry_rows'] == reopen['warm_body_bytes'] == 0
                assert reopen['cold_metrics']['history_entry_rows'] == 100
                assert reopen['cold_metrics']['history_candidate_rows'] <= 101
                assert reopen['cold_metrics']['history_entry_body_bytes'] <= 1024**2
                assert reopen['update_page_rows'] == 100
                assert reopen['cache']['bytes'] <= 16 * 1024**2
                assert reopen['read_budget']['bytes'] <= 256 * 1024**2
    elif binary == 'kernel_lifecycle_probe':
        assert rows[-1]['phase'] == 'closed'
        assert rows[-1]['read_budget_after_close']['bytes'] == 0
        case = rows[0]['case']
        phases = {row['phase'] for row in rows}
        if case == 'b05':
            checkpoints = [row for row in rows if row['phase'] == 'round_checkpoint']
            expected = [10, 100, 500] if full else [3, 10]
            assert [row['measured_rounds'] for row in checkpoints] == expected
            for row in checkpoints:
                count = row['measured_rounds']
                assert row['database']['rounds'] == count + 1
                assert row['database']['round_inputs'] == count * 2 + 1
                assert row['database']['replay_edges'] == count
                assert row['database']['provider_state_bytes'] == rows[0]['provider_state_bytes_per_round'] * (count + 1)
        elif case == 'b06':
            assert {'fixture', 'wide_forks', 'deep_forks', 'shared_live_output', 'delete_and_gc'} <= phases
            shared = next(row for row in rows if row['phase'] == 'shared_live_output')
            count = 200 if full else 8
            assert shared['branches_read'] == shared['reliable_deliveries'] == shared['business_feed_notifications'] == count
            assert shared['content_subscribers_admitted'] + shared['content_subscribers_busy'] == count
            if full:
                assert rows[0]['history_entries'] == 100000
                assert shared['content_subscribers_admitted'] == 64
        elif case == 'b07':
            assert {'log_sealed', 'mixed_sealed', 'parent_succeeded_background_open',
                    'background_succeeded_capture_interrupted', 'retention_complete'} <= phases
            if full:
                assert rows[0]['log_mib'] >= 256
            log = next(row for row in rows if row['phase'] == 'log_sealed')
            assert log['range_verification']['bytes'] == rows[0]['log_mib'] * 1024**2
            assert len([row for row in rows if row['phase'] == 'retention_source']) == 5
    elif binary == 'kernel_export_probe':
        assert len(rows) == 1
        assert rows[0]['all_exports_verified']
        assert rows[0]['concurrent_exports'] == len(rows[0]['exports']) == 2
        assert rows[0]['shared_export_overlap_ms'] > 0
        assert all(export['verification']['verified'] for export in rows[0]['exports'])
        reads = rows[0]['read_transactions']
        assert reads['started'] == reads['completed'] > 0
        assert reads['abandoned_end_unobserved'] == reads['active_measurement_scopes'] == 0
        assert sum(bucket['count'] for bucket in reads['completed_duration_histogram']) == reads['completed']
        if full:
            assert rows[0]['control']['samples'] >= 5
            assert all(row['samples_wholly_during_export'] >= 5 for row in rows[0]['control']['per_export'])
    elif binary == 'content_delivery_probe':
        assert len(rows) == 2
        assert [row['observers'] for row in rows] == [1, 8]
        for row in rows:
            assert row['logical_appends'] == 100
            assert row['source_bytes'] == 102400
            assert row['samples'] == row['observers'] * 100
            assert row['client_visible_busy_errors'] == row['client_reconnections'] == 0
            integrity = row['observer_integrity']
            assert len(integrity) == row['observers']
            assert len({observer['blake3'] for observer in integrity}) == 1
            for observer in integrity:
                assert observer['received_bytes'] == observer['final_cursor'] == 102400
                assert observer['marker_samples'] == 100
                assert observer['pages'] > 0
                assert observer['client_visible_busy_errors'] == observer['client_reconnections'] == 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--binary-dir', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--mode', choices=['smoke', 'full'], required=True)
    parser.add_argument('--without-api', action='store_true')
    args = parser.parse_args()
    args.root = args.root.resolve(strict=True)
    args.binary_dir = args.binary_dir.resolve(strict=True)
    args.out.mkdir(parents=True, exist_ok=False)
    runner = Path(__file__).resolve()
    shutil.copy2(runner, args.out / 'runner.py')
    selected = cases(args.mode)
    if args.without_api:
        selected = [case for case in selected if case[1] != 'content_delivery_probe']
    source_paths = output(['rg', '--files', 'crates', 'apps', 'tools', '-g', '*.rs', '-g', '*.py', '-g', 'Cargo.toml'], args.root).splitlines()
    source_paths += ['Cargo.toml', 'Cargo.lock']
    fingerprints = {
        path: hashlib.sha256((args.root / path).read_bytes()).hexdigest()
        for path in sorted(set(source_paths))
    }
    source_digest = hashlib.sha256(json.dumps(fingerprints, sort_keys=True).encode()).hexdigest()
    (args.out / 'source-fingerprints.json').write_text(json.dumps(fingerprints, indent=2) + '\n')
    (args.out / 'git-status.txt').write_text(output(['git', 'status', '--short'], args.root) + '\n')
    save_process_activity(args.root, args.out / 'processes-before.json')
    (args.out / 'filesystem-volumes.txt').write_text(output(['df', '-h', str(args.binary_dir), '/tmp'], args.root) + '\n')
    environment = {
        'mode': args.mode, 'started_utc': now(), 'platform': platform.platform(),
        'machine': platform.machine(), 'logical_cpus': output(['sysctl', '-n', 'hw.logicalcpu'], args.root),
        'physical_memory_bytes': output(['sysctl', '-n', 'hw.memsize'], args.root),
        'rustc': output(['rustc', '--version'], args.root), 'cargo': output(['cargo', '--version'], args.root),
        'git_commit': output(['git', 'rev-parse', 'HEAD'], args.root),
        'runner_sha256': hashlib.sha256(runner.read_bytes()).hexdigest(),
        'source_files_sha256': source_digest,
        'target_volume_free_bytes_before': shutil.disk_usage(args.binary_dir).free,
        'system_tmp_free_bytes_before': shutil.disk_usage('/tmp').free,
        'provider_enabled': False, 'os_cache_evicted': False,
        'measurement': 'serial child processes; owned TempDir databases; smoke timings are behavior-only; full timings require no overlapping build/test/probe',
        'host_activity': 'unrelated applications are not stopped; process snapshots and per-case load averages describe background load; a fully idle host is not assumed',
        'cases': len(selected), 'binaries': {},
    }
    for binary in sorted({binary for _, binary, _ in selected}):
        path = args.binary_dir / binary
        environment['binaries'][binary] = {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
    (args.out / 'environment.json').write_text(json.dumps(environment, indent=2) + '\n')
    content = []
    passed = []
    with (args.out / 'runs.jsonl').open('x') as manifest:
        for label, binary, params in selected:
            command = [str(args.binary_dir / binary), *params]
            record = {'case': label, 'command': command, 'started_utc': now(),
                      'load_average_before': list(os.getloadavg())}
            started = time.monotonic()
            output_path = args.out / f'{label}.jsonl'
            error_path = args.out / f'{label}.stderr.log'
            try:
                with output_path.open('x') as target, error_path.open('x') as error:
                    process = subprocess.run(command, cwd=args.root, stdout=target, stderr=error, timeout=900)
                record['returncode'] = process.returncode
                rows = [json.loads(line) for line in output_path.read_text().splitlines()]
                record['json_records'] = len(rows)
                assert process.returncode == 0, f'exit {process.returncode}; see {error_path}'
                validate(binary, rows, args.mode == 'full')
                if binary == 'kernel_content_probe':
                    content.append((label, rows[0]))
                record['validation_passed'] = True
                passed.append(label)
            except Exception as error:
                record.update(validation_passed=False, failure=f'{type(error).__name__}: {error}')
            record.update(finished_utc=now(), elapsed_s=time.monotonic() - started,
                          load_average_after=list(os.getloadavg()))
            manifest.write(json.dumps(record, sort_keys=True) + '\n')
            manifest.flush()
            print(json.dumps(record, sort_keys=True), flush=True)
            if not record['validation_passed']:
                save_process_activity(args.root, args.out / 'processes-after.json')
                environment.update(finished_utc=now(), failed_case=label)
                (args.out / 'environment.json').write_text(json.dumps(environment, indent=2) + '\n')
                raise SystemExit(f'{label} failed; raw results preserved in {args.out}')
    save_process_activity(args.root, args.out / 'processes-after.json')
    gates = []
    if args.mode == 'full':
        for repeat, sources, observers in itertools.product([1, 2, 3], [1, 8, 32], [1, 8]):
            values = {
                row['slicing']: row['content_increment_page_bytes']
                for label, row in content
                if label.startswith('b01-text-') and label.endswith(f'-r{repeat}') and row['sources'] == sources and row['observers_per_source'] == observers
            }
            small, large = values['small'], values['large']
            allowed = 0.15 * large + 65536
            gates.append({'repeat': repeat, 'sources': sources, 'observers': observers,
                          'small_page_bytes': small, 'large_page_bytes': large,
                          'absolute_difference_bytes': abs(small - large),
                          'allowed_difference_bytes': allowed, 'passed': abs(small - large) <= allowed})
    summary = {'mode': args.mode, 'completed_cases': passed, 'case_count': len(passed),
               'packet_slicing_space_gates': gates, 'all_space_gates_passed': all(gate['passed'] for gate in gates)}
    (args.out / 'gates.json').write_text(json.dumps(summary, indent=2) + '\n')
    final_source_paths = output(['rg', '--files', 'crates', 'apps', 'tools', '-g', '*.rs', '-g', '*.py', '-g', 'Cargo.toml'], args.root).splitlines()
    final_source_paths += ['Cargo.toml', 'Cargo.lock']
    final_fingerprints = {
        path: hashlib.sha256((args.root / path).read_bytes()).hexdigest()
        for path in sorted(set(final_source_paths))
    }
    (args.out / 'source-fingerprints-after.json').write_text(json.dumps(final_fingerprints, indent=2) + '\n')
    final_binaries = {
        name: hashlib.sha256(Path(binary['path']).read_bytes()).hexdigest()
        for name, binary in environment['binaries'].items()
    }
    source_unchanged = fingerprints == final_fingerprints
    binaries_unchanged = all(final_binaries[name] == binary['sha256']
                             for name, binary in environment['binaries'].items())
    environment.update(finished_utc=now(), target_volume_free_bytes_after=shutil.disk_usage(args.binary_dir).free,
                       system_tmp_free_bytes_after=shutil.disk_usage('/tmp').free,
                       source_files_unchanged_during_run=source_unchanged,
                       binaries_unchanged_during_run=binaries_unchanged,
                       final_binary_sha256=final_binaries)
    (args.out / 'environment.json').write_text(json.dumps(environment, indent=2) + '\n')
    assert source_unchanged, 'sources changed during the matrix; do not call these final-source results'
    assert binaries_unchanged, 'probe binaries changed during the matrix'
    assert summary['all_space_gates_passed'], 'packet slicing exceeded the documented tolerance'


if __name__ == '__main__':
    main()
