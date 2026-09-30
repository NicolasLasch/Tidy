# Change file permissions

Context: User requests ordinary file read/write/execute permissions.

Workflow: discover exact regular files; translate requested access to ordinary Unix mode; preserve owner read access; show before and after in approval; no ownership, ACLs, elevation or special bits.

Only discovered indexed IDs, paths, byte lengths, modification dates and saved excerpts. Missing text or hashes means unknown.

Clarify ambiguity; stop at budget or cancellation; do not invent missing evidence; every change requires separate approval.
