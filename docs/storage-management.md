# Storage Management

The Desktop store lives under `%LOCALAPPDATA%\io.cakesplitter.desktop` and
uses a single-process lock, SQLite WAL, full synchronous writes, checksums,
bounded quarantine, and an epoch fence for Clear All.

Limits are deliberately explicit:

- 32 MiB per serialized task record;
- 64 nonterminal records and 64 startup-recovery records;
- 500 ordinary terminal-history records;
- 500 checkpoint-bearing terminal records;
- 20 quarantined records, with 64 KiB data and 1,000 bytes of reason per row;
- 10 retained failure entries per task; and
- 20 preflight warnings per task.

Rejected admissions occur before persistence and execution allocation. Corrupt
or future-schema rows are quarantined with bounded evidence. Clear Failed
History cleans identity-owned incomplete outputs before removing failed rows;
it does not sweep arbitrary files in a selected directory.
