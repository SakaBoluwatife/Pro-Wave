# Pro Wave development notes

The running record of the autonomous development pass: what was done, what is left, known issues and decisions. `docs/LOGIC-WIREFRAME.md` stays the feature map.

Test results here are automated unless a line says otherwise. Nothing in this file has been checked by ear or by hand unless it says so.

## Audit (2026-11-04, from commit 265d1a4)

- Workspace of five crates: `prowave-app` (54,800 lines), `prowave-engine` (15,500), `prowave-host` (3,800), `prowave-project` (1,000), `xtask` (1,100).
- All tests passed in a clean run: 711 passed, 0 failed, 12 ignored. One earlier run had 1 failure while other work loaded the machine: an intermittent wall-clock limit.
- Biggest risks found: no crash recovery of unsaved work; output clamped at 0 dBFS; plugin parameters not synchronised with their lanes; no process isolation; the mix runs on one thread; colours written into each screen.
- Large files that would pay for splitting, only when there is a reason to touch them: `tracks_area.rs` (4,100 lines), `model.rs` (3,200), `mixer.rs` (3,100).

## Decisions

- Autosave copies media on a background thread and never touches the project file. It writes per process, so two running copies do not take each other's recovery file for a crash.

## Done

See the git log for detail. By stage:

### Stage 2: reliability

(in progress)

## Remaining and known issues

See the sections above and `docs/LOGIC-WIREFRAME.md` section 0.
