"""Capture isolated fixture diagnostics after their guardian has stopped every writer."""
from contextlib import closing
import json
import os
from pathlib import Path
import shutil
import sqlite3


def write_json(path, value, **options):
    # A killed writer must not leave a truncated verdict or observation summary.
    temporary = path.with_name(f".{path.name}.tmp")
    temporary.write_text(json.dumps(value, indent=2, **options) + "\n")
    temporary.replace(path)


def register(root, evidence):
    manifest = os.environ.get("FRACTAL_TEST_EVIDENCE_MANIFEST")
    if not manifest:
        raise RuntimeError("fixture evidence requires its process guardian")
    evidence.mkdir(parents=True, exist_ok=True)
    with open(manifest, "a") as output:
        output.write(json.dumps({"root": str(root), "evidence": str(evidence)}) + "\n")


def capture(root, evidence, final=False):
    """Independent files must survive an unreadable spool or a failed API trace."""
    evidence.mkdir(parents=True, exist_ok=True)
    errors = []
    logs = [root / "daemon.log", root / "omp-channel-stderr.log", root / "omp-channel-exit.txt",
            root / "ws/native-starts.jsonl", *sorted(root.rglob("driver.log")),
            *sorted(root.rglob("driver-api-warnings.log")),
            *sorted((root / "ws").glob("receipts-*.jsonl"))]
    driver = 0
    for log in logs:
        if not log.is_file():
            continue
        name = log.name
        if name.startswith("driver"):
            name = f"driver-{driver}.log"
            driver += 1
        try:
            shutil.copyfile(log, evidence / name)
        except OSError as error:
            errors.append(f"{log}: {error}")
    if not (evidence / "daemon.log").exists():
        (evidence / "daemon.log").write_text("Daemon did not create a log before interruption.\n")
    if not driver:
        (evidence / "driver-0.log").write_text("No driver warning log was created.\n")
    summaries = []
    for database in sorted(root.rglob("st-harness-events.sqlite")):
        row = {"path": str(database.relative_to(root))}
        try:
            with closing(sqlite3.connect(f"{database.as_uri()}?mode=ro", uri=True)) as connection:
                connection.execute("BEGIN")
                row["pending"] = connection.execute("SELECT sequence,kind FROM events ORDER BY sequence").fetchall()
                row["metadata"] = connection.execute("SELECT key,value FROM metadata WHERE key='runtime' OR key LIKE 'provider-runtime:%'").fetchall()
                row["states"] = [{"kind": kind, "state": json.loads(body).get("state"),
                                  "incarnation": json.loads(body).get("incarnation")}
                                 for kind, body in connection.execute("SELECT kind,body FROM snapshots WHERE kind='harness-state'")]
        except (sqlite3.Error, ValueError) as error:
            row["error"] = str(error)
            errors.append(f"{database}: {error}")
        summaries.append(row)
    summary = evidence / "observation-outboxes.json"
    if final and summary.exists():
        summary = evidence / "observation-outboxes-after-stop.json"
    write_json(summary, summaries)
    if errors:
        write_json(evidence / "capture-errors.json", errors)
    return errors


def finish(manifest, code):
    errors = []
    for line in manifest.read_text().splitlines():
        entry = json.loads(line)
        root, evidence = Path(entry["root"]), Path(entry["evidence"])
        try:
            result_path = evidence / "result.json"
            if code:
                capture(root, evidence, final=True)
                write_json(evidence / "guardian.json", {"exit": code})
                if not result_path.exists():
                    write_json(result_path, {"verdict": "fail", "failure": "fixture ended before verdict",
                                             "guardian_exit": code})
        except Exception as error:
            errors.append(f"{evidence}: {error}")
        finally:
            if root.exists():
                shutil.rmtree(root)
    if errors:
        raise RuntimeError("; ".join(errors))
