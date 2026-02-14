These fixtures are curated from exploratory signature dumps and are used by automated tests.

- `snapshot_*.txt`: valid chronological snapshots for one system evolving over time.
- `bad_*.txt`: malformed snapshots used to validate parser errors.

The parser expects tab-delimited columns in this order:

1. signature id
2. group
3. site type (optional)
4. name (optional)
5. scan percent (optional, `%` suffix)
6. distance (ignored by parser)
