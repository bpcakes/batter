# Last-good snapshot file, format v1 (`BFSN`)

Status: normative for `batter-flags` with the `postgres` feature. The parser
golden test `the_encoder_and_parser_match_the_documented_layout` assembles a
file from this document, field by field, and requires the encoder to produce
exactly those bytes and the parser to read them back.

The file is a cache of the control plane's last publication. A reader MUST
treat every file as untrusted input. Deleting the file is always safe.

## Layout

All integers are unsigned and big-endian. Text fields are UTF-8 and carry a
length prefix of the stated width; a reader MUST refuse text that is not valid
UTF-8.

| Field | Size | Meaning |
| --- | --- | --- |
| magic | 4 | MUST be the ASCII bytes `BFSN` |
| format version | 1 | MUST be `1` |
| scope label | 2-byte length + text | MUST be the settings prefix, `:`, and the writing process kind, e.g. `FLAG_:serving` |
| catalog digest | 32 | the writer's `CatalogDigest` |
| manifest digest | 32 | the writer's `ManifestDigest` |
| snapshot version | 8 | the control-plane version the entries were published at |
| entry count | 4 | the number of entries that follow |
| entries | variable | `entry count` entries, as below |
| trailer | 32 | SHA-256 of every preceding byte |

Each entry:

| Field | Size | Meaning |
| --- | --- | --- |
| key | 1-byte length + text | the flag or kill-switch key |
| kind | 1-byte length + text | `release`, `mode` or `switch` |
| value type | 1-byte length + text | `toggle`, `enum` or `switch` |
| fingerprint | 32 | the `DeclarationFingerprint` the entry was written for |
| updated version | 8 | the version that last changed the row |
| state | 2-byte length + text | the row's state in state encoding 1, or empty for an entry written without a value |

A file MUST NOT exceed 1 MiB (1 048 576 bytes). A writer MUST NOT write a key
twice. A state of length zero means the entry carries no value; no state
encoding produces an empty state, so it is never ambiguous.

The file name MUST end with `.bfsn`. The temporary name is the file name
followed by `.tmp`, and the probe the directory check creates is the file
name followed by `.probe`; neither can be another valid file name.

## Reading

A reader MUST refuse, publishing nothing from the file:

- a file larger than 1 MiB (`TooLarge`);
- a file whose structure ends before its fields do (`Truncated`);
- bytes after the trailer (`TrailingBytes`);
- a trailer that is not the SHA-256 of the preceding bytes (`Tampered`);
- another magic or non-UTF-8 text (`Malformed`), or another format version
  (`UnsupportedVersion`);
- a scope label other than its own (`Foreign`);
- a repeated key (`Rejected`).

A reader MUST read only a regular file, and MUST NOT read more than one byte
past the limit before refusing it, so a FIFO, a device or an oversized file
neither blocks nor exhausts the reader. An entry without a value MUST be
treated as absent: the reader serves the switch's safe state or the declared
base for it.

A reader MUST NOT refuse a file because its catalog or manifest digest
differs from its own: entries are matched one by one. An entry whose key the
reader does not declare, or whose declaration its process kind does not own,
MUST be ignored. A declared, owned entry whose fingerprint equals the running
declaration's MUST apply as `Source::PersistedFile`, subject to the same
judgement as a control-plane row; one whose fingerprint differs MUST be held
with cause `entry_type_mismatch`.

None of these refusals is a startup failure. A reader reports the outcome
through `RegisteredFlags::startup_file()`: `Disabled`, `Missing`,
`Published { version }` or `Unusable(problem)`. Opening the directory, by
contrast, fails startup when the directory is missing, is not a directory or
does not let this process create and remove a file.

## Writing

A writer writes only values its process published from a control-plane read,
for keys its process declares and owns, under the declaration's kind and
value type. A held entry is written with a value only when it carries an
engaged kill switch from an earlier control-plane publication, as that state
at the version it was engaged since; any other held entry, including one whose
last state came from a file, MUST be written without a value. File, declared
and test state MUST NOT be written.

A writer writes whenever the published control-plane version differs from the
version it last wrote, and retries a waiting write every run, whatever that
run read. Its first write replaces any file; later writes skip a file whose
version is newer. A failure before the rename leaves the file unchanged
(`Io`); a failed directory sync after it leaves the new file in place with
unconfirmed durability (`NotDurable`), and the write is retried.

A writer MUST hold an exclusive lock on the directory (`flock` through
`File::try_lock`) for the whole write and MUST fail without writing when the
lock is held (`Busy`). Under the lock it removes residue at the fixed
temporary name `<file>.tmp`, creates it with `O_EXCL` and mode `0600`, writes
the whole file, syncs it, renames it over the final name and syncs the
directory.

## Verification labels

- Directory `fsync` after the rename is relied on for the rename's
  durability. It is verified on Linux only; on macOS, where `fsync` does not
  flush the drive cache, durability across power loss is unverified.
- `flock` on tmpfs and overlayfs directories is unverified.
