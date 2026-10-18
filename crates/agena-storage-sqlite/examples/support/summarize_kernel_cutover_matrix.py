#!/usr/bin/env python3
"""Summarize preserved raw cutover records without pooling unlike samples."""

import argparse
import json
from pathlib import Path
import statistics


def at(row, path):
    for field in path.split('.'):
        if not isinstance(row, dict):
            return None
        row = row.get(field)
    return row


def distribution(values):
    available = [value for value in values if isinstance(value, (int, float)) and not isinstance(value, bool)]
    return {
        'values': values, 'available_samples': len(available),
        'minimum': min(available) if available else None,
        'median': statistics.median(available) if available else None,
        'maximum': max(available) if available else None,
    }


def metrics(rows, fields):
    return {field: distribution([at(row, field) for row in rows]) for field in fields}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('result', type=Path)
    args = parser.parse_args()
    runs = [json.loads(line) for line in (args.result / 'runs.jsonl').read_text().splitlines()]
    assert runs and all(row.get('validation_passed') for row in runs)
    data = {row['case']: [json.loads(line) for line in (args.result / f"{row['case']}.jsonl").read_text().splitlines()] for row in runs}
    groups = {}
    for name, records in data.items():
        if name.startswith(('b01-', 'b02-log-')):
            row = records[0]
            key = f"B01/B02 kind={row['kind']} sources={row['sources']} observers={row['observers_per_source']} slicing={row['slicing']}"
            groups.setdefault(key, []).append(row)
        elif name.startswith(('b03-', 'b04-')) or name == 'b03':
            row = records[0]
            key = f"B03/B04 recent={row['recent_entries']} cold_MiB={row['cold_mib']}"
            reopens = [reopen for sample in row['runs'] for reopen in sample['reopens']]
            summary = {
                'fixture': row['fixture'], 'reader_processes': len(row['runs']),
                'reopens_per_process': [len(sample['reopens']) for sample in row['runs']],
                'warm_reads_total': sum(reopen['warm_reads'] for reopen in reopens),
                'warm_queries_total': sum(reopen['warm_queries'] for reopen in reopens),
                'process_peak_rss_sampled_bytes': [sample['rss_peak_sampled_bytes'] for sample in row['runs']],
                'individual_reopen_metrics': metrics(reopens, [
                    'open_ms', 'cold_page_ms', 'warm_p50_ms', 'warm_p95_ms',
                    'send_ms', 'cancel_ms', 'metadata_update_ms',
                    'cold_metrics.history_queries', 'cold_metrics.history_candidate_rows',
                    'cold_metrics.history_entry_rows', 'cold_metrics.history_entry_body_bytes',
                    'cache.bytes', 'read_budget.bytes', 'usage_warm.resident_bytes',
                ]),
            }
            groups[key] = summary
        elif name.startswith('b05-') or name == 'b05':
            for row in records:
                if row['phase'] == 'round_checkpoint':
                    groups.setdefault(f"B05 rounds={row['measured_rounds']}", []).append(row)
        elif name.startswith('b06-') or name == 'b06':
            for phase in ['wide_forks', 'deep_forks', 'shared_live_output', 'delete_and_gc', 'closed']:
                row = next(row for row in records if row['phase'] == phase)
                groups.setdefault(f'B06 {phase}', []).append(row)
        elif name.startswith('b07-') or name == 'b07':
            for phase in ['log_sealed', 'mixed_sealed', 'parent_succeeded_background_open',
                          'background_succeeded_capture_interrupted', 'retention_complete', 'closed']:
                row = next(row for row in records if row['phase'] == phase)
                groups.setdefault(f'B07 {phase}', []).append(row)
            for row in records:
                if row['phase'] == 'retention_source':
                    retention = row['retention']
                    groups.setdefault(f"B07 retention {retention['stream']['kind']} ordinal={retention['stream']['ordinal']}", []).append(retention)
        elif name.startswith('b08-') or name == 'b08':
            groups.setdefault(f"B08 {'large' if '-large-' in name else 'small'}", []).append(records[0])
            for export in records[0]['exports']:
                groups.setdefault(f"B08 {'large' if '-large-' in name else 'small'} sink={export['sink']}", []).append(export)
            for cohort in records[0]['control']['per_export']:
                groups.setdefault(f"B08 {'large' if '-large-' in name else 'small'} fairness sink={cohort['sink']}", []).append(cohort)
        elif name.startswith('b02-api'):
            for row in records:
                groups.setdefault(f"B02 API observers={row['observers']}", []).append(row)
    summaries = {}
    common = ['sampling.rss_peak_sampled_bytes', 'diagnostics.read_budget.peak_bytes',
              'diagnostics.read_budget.peak_source_bytes', 'diagnostics.read_budget.rejected',
              'database.database_page_bytes', 'database.content_blocks', 'database.content_block_bytes',
              'database.files.wal_bytes']
    for key, rows in groups.items():
        if isinstance(rows, dict):
            summaries[key] = rows
            continue
        if key.startswith('B01/'):
            fields = ['accepted_appends', 'blocks', 'stored_body_bytes', 'content_increment_page_bytes',
                      'checkpoint_transactions', 'command_results_during_content', 'command_transactions_during_content',
                      'terminal_finish_ms', 'elapsed_ms', 'rss_peak_sampled_bytes',
                      'append_call_to_accepted.p95_upper_ms', 'append_call_to_range_read.p95_upper_ms',
                      'append_call_to_committed_observation.p95_upper_ms', 'reader_busy_retries',
                      'os_disk_written_bytes_content_through_close', 'wal_file_bytes_at_final_observation']
        elif key.startswith('B02 API'):
            fields = ['samples', 'source_bytes', 'delivery_p50_ms', 'delivery_p95_ms', 'delivery_max_ms',
                      'client_visible_busy_errors', 'client_reconnections', 'server_internal_busy_retries']
        elif key.startswith('B05'):
            fields = common + ['elapsed_ms', 'interval_ms', 'interval_rounds',
                      'interval_io.disk_written_bytes', 'database.rounds', 'database.round_inputs',
                      'database.replay_edges', 'database.provider_state_bytes', 'database.freelist_pages']
        elif key.startswith('B06'):
            fields = common + ['elapsed_ms', 'fork_p50_ms', 'fork_p95_ms', 'fork_edges_added',
                      'database_after.session_edges', 'database_after.segment_entries', 'database_after.entries',
                      'database_after.database_page_bytes', 'database.session_edges', 'database.segment_entries',
                      'content_subscribers_admitted', 'content_subscribers_busy', 'branches_read',
                      'notification_p50_ms', 'notification_p95_ms', 'reliable_deliveries',
                      'deliveries_ms', 'origin_delete_ms', 'branch_delete_ms', 'total_io.disk_written_bytes']
        elif key.startswith('B07 retention '):
            fields = ['actual_retained_start', 'requested_prefix', 'blocks_removed',
                      'bounded_reclamation_calls', 'elapsed_ms', 'io.disk_written_bytes',
                      'stream.committed', 'stream.capture_loss.committed_bytes']
        elif key.startswith('B07'):
            fields = common + ['elapsed_ms', 'range_verification.bytes', 'range_verification.events',
                      'range_verification.max_page_payload_bytes', 'range_verification.elapsed_ms',
                      'terminal_range_verification.records_read', 'raw_log_range_verification.bytes',
                      'busy_retries', 'database_before.database_page_bytes', 'database_before.content_block_bytes',
                      'database_after.database_page_bytes', 'database_after.content_block_bytes',
                      'database_after.freelist_pages', 'total_io.disk_written_bytes']
        else:
            fields = ['elapsed_s', 'concurrent_exports', 'shared_export_overlap_ms', 'report.bytes', 'report.entries', 'report.records',
                      'verification.max_line_bytes', 'control.samples', 'control.reply_p95_ms',
                      'control.reply_max_ms', 'control.cancel_p95_ms', 'control.cancel_max_ms',
                      'control.history_p95_ms', 'samples_wholly_during_export', 'append_p95_ms', 'reply_p95_ms',
                      'send_p95_ms', 'cancel_p95_ms', 'history_p95_ms', 'rss_peak_sampled_bytes', 'wal_peak_sampled_bytes',
                      'read_budget.peak_bytes', 'max_sink_write_bytes', 'database_bytes',
                      'read_transactions.started', 'read_transactions.completed',
                      'read_transactions.abandoned_end_unobserved',
                      'read_transactions.active_measurement_scopes',
                      'read_transactions.peak_active_measurement_scopes',
                      'read_transactions.completed_total_ns', 'read_transactions.completed_mean_ns',
                      'read_transactions.completed_p50_upper_us', 'read_transactions.completed_p95_upper_us',
                      'read_transactions.completed_max_ns']
        values = metrics(rows, fields)
        summaries[key] = {'runs': len(rows), 'metrics': {field: value for field, value in values.items() if value['available_samples']}}
    result = {
        'completed_top_level_runs': len(runs),
        'interpretation': 'min/median/max are across independent runs or labeled reopens; inner p95 values remain individual-run p95s and are not pooled; null is unavailable, not zero; timings require reading the run environment and scope limitations',
        'groups': summaries,
    }
    target = args.result / 'summary.json'
    with (args.result / 'summarizer.py').open('x') as source:
        source.write(Path(__file__).read_text())
    with target.open('x') as file:
        json.dump(result, file, indent=2)
        file.write('\n')
    print(target)


if __name__ == '__main__':
    main()
