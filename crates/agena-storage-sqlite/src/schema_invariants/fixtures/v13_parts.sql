-- Historical version-13 part triggers, before the parts-integrity audit.

CREATE TRIGGER IF NOT EXISTS agena_parts_identity_immutable BEFORE UPDATE OF part_id, kind, role, origin_session_id, created_at_ms ON agena_parts WHEN OLD.part_id != NEW.part_id OR OLD.kind != NEW.kind OR OLD.role != NEW.role OR OLD.origin_session_id IS NOT NEW.origin_session_id OR OLD.created_at_ms != NEW.created_at_ms BEGIN SELECT RAISE(ABORT, 'part identity is immutable'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_state_valid_insert BEFORE INSERT ON agena_parts WHEN NEW.state NOT IN ('pending', 'in_progress', 'completed', 'failed', 'cancelled') OR NEW.visibility NOT IN ('both', 'user', 'ai') BEGIN SELECT RAISE(ABORT, 'invalid part state or visibility'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_state_valid_update BEFORE UPDATE OF state, visibility ON agena_parts WHEN NEW.state NOT IN ('pending', 'in_progress', 'completed', 'failed', 'cancelled') OR NEW.visibility NOT IN ('both', 'user', 'ai') BEGIN SELECT RAISE(ABORT, 'invalid part state or visibility'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_lifecycle_shape_valid BEFORE INSERT ON agena_parts WHEN NEW.started_at_ms < 0 OR NEW.created_at_ms < 0 OR NEW.updated_at_ms < NEW.created_at_ms BEGIN SELECT RAISE(ABORT, 'invalid part lifecycle shape'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_lifecycle_shape_update_valid BEFORE UPDATE OF updated_at_ms ON agena_parts WHEN NEW.updated_at_ms < OLD.updated_at_ms BEGIN SELECT RAISE(ABORT, 'part updated_at cannot move backwards'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_retry_requires_revision_bump BEFORE UPDATE OF state ON agena_parts WHEN NEW.state = 'in_progress' AND OLD.state IN ('failed', 'cancelled') AND NEW.revision <= OLD.revision BEGIN SELECT RAISE(ABORT, 'retrying a part must bump its revision'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_revision_monotonic BEFORE UPDATE OF revision ON agena_parts WHEN NEW.revision < OLD.revision BEGIN SELECT RAISE(ABORT, 'part revision cannot decrease'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_content_is_json BEFORE INSERT ON agena_parts WHEN json_valid(NEW.content) != 1 BEGIN SELECT RAISE(ABORT, 'part content must be a JSON document'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_run_marker_is_batch_root BEFORE INSERT ON agena_parts WHEN NEW.kind = 'run' AND (NEW.run_id IS NOT NULL OR NEW.parent_part_id IS NOT NULL) BEGIN SELECT RAISE(ABORT, 'run marker parts must be the root of their batch'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_run_marker_root_immutable BEFORE UPDATE OF run_id, parent_part_id ON agena_parts WHEN OLD.kind = 'run' AND (NEW.run_id IS NOT NULL OR NEW.parent_part_id IS NOT NULL) BEGIN SELECT RAISE(ABORT, 'run marker parts must stay the root of their batch'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_run_marker_requires_run_kind BEFORE INSERT ON agena_parts WHEN NEW.kind = 'run' AND json_type(NEW.content, '$.run_kind') IS NULL BEGIN SELECT RAISE(ABORT, 'run marker part requires run_kind in content'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_run_marker_terminal_abort_reason BEFORE INSERT ON agena_parts WHEN NEW.kind = 'run' AND NEW.state IN ('completed', 'failed', 'cancelled') AND json_type(NEW.content, '$.abort_reason') IS NULL BEGIN SELECT RAISE(ABORT, 'terminal run marker requires abort_reason'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_run_marker_terminal_abort_reason_update BEFORE UPDATE OF state ON agena_parts WHEN NEW.kind = 'run' AND NEW.state IN ('completed', 'failed', 'cancelled') AND json_type(NEW.content, '$.abort_reason') IS NULL BEGIN SELECT RAISE(ABORT, 'terminal run marker requires abort_reason'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_run_id_references_run_marker BEFORE INSERT ON agena_parts WHEN NEW.run_id IS NOT NULL AND NOT EXISTS ( SELECT 1 FROM agena_parts run WHERE run.part_id = NEW.run_id AND run.kind = 'run' ) BEGIN SELECT RAISE(ABORT, 'part run_id must reference a run marker part'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_run_id_references_run_marker_update BEFORE UPDATE OF run_id ON agena_parts WHEN NEW.run_id IS NOT NULL AND NOT EXISTS ( SELECT 1 FROM agena_parts run WHERE run.part_id = NEW.run_id AND run.kind = 'run' ) BEGIN SELECT RAISE(ABORT, 'part run_id must reference a run marker part'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_parent_references_part BEFORE INSERT ON agena_parts WHEN NEW.parent_part_id IS NOT NULL AND ( NEW.parent_part_id = NEW.part_id OR NOT EXISTS (SELECT 1 FROM agena_parts parent WHERE parent.part_id = NEW.parent_part_id) ) BEGIN SELECT RAISE(ABORT, 'part parent_part_id must reference an existing part'); END;

CREATE TRIGGER IF NOT EXISTS agena_parts_parent_references_part_update BEFORE UPDATE OF parent_part_id ON agena_parts WHEN NEW.parent_part_id IS NOT NULL AND ( NEW.parent_part_id = NEW.part_id OR NOT EXISTS (SELECT 1 FROM agena_parts parent WHERE parent.part_id = NEW.parent_part_id) ) BEGIN SELECT RAISE(ABORT, 'part parent_part_id must reference an existing part'); END;
