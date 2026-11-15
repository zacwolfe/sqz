# sqz token-saving audit

What this document is: the ten fixes that came out of the 260920 audit of the
compression pipeline, what each one changed, and what it measured on a fixed
corpus. Numbers in the tables come from the commands in
[Measurement](#measurement), run on this machine against the checked-out tree.
No number here is a projection or a worker report.

## Measurement

```bash
CORPUS=/tmp/sqzcorpus
for f in api.json cargo_test.txt docker.txt gitdiff.txt ls.txt migration.sql \
         npm.txt pytest.txt server.log tsc.txt; do
  SQZ_NO_DEDUP=1 ./target/debug/sqz compress --no-cache < "$CORPUS/$f" | wc -c
done
```

`SQZ_NO_DEDUP=1` and `--no-cache` keep the dedup cache out of the result, so the
byte counts are the pipeline and nothing else. With no `--mode`, `compress` on
stdin takes the shell-hook path: dedup lookup, then a per-command formatter, then
the generic pipeline. The label is `stdin`, so no per-command formatter matches
and every fixture is measured by the generic pipeline, which is the path an agent
meets for anything sqz has no formatter for. The corpus holds ten fixtures, one
per output shape: an API response, a cargo test run, a docker build, an eight-file
git diff, an `ls -la`, a SQL migration, an `npm install`, a pytest run, a server
log, and a `tsc` run. Each one is generated to match the real tool's line format
with indexed identifiers (`dir_0`, `pkg-0`, `test_0`, `Comp0.tsx`), which is what
makes repeated-row shapes visible and comparable across commits. Total input is
221,746 bytes. One eleventh fixture is used only by fix 10: a 2687-byte real
capture of a rate-limited `curl` transfer, which carries `\r` redraw frames and
is the one shape the ten generated fixtures do not have. It is reported in fix
10's row and is not part of the total.

Three comparison points:

- `series` is `5441bab`, the commit before the first fix in this series.
- `fix05` is `0fc5eaf`, the baseline the measurement harness was pinned to and
  the first commit that keeps content only when the output declares the loss.
- `HEAD` is the last row of the fix table below.

| fixture | input | series | reduction | fix05 | reduction | HEAD | reduction |
|---|---:|---:|---:|---:|---:|---:|---:|
| api.json | 26533 | 10979 | 58.6% | 9139 | 65.6% | 9139 | 65.6% |
| cargo_test.txt | 8418 | 8418 | 0.0% | 8418 | 0.0% | 283 | 96.6% |
| docker.txt | 6482 | 3886 | 40.0% | 3886 | 40.0% | 3630 | 44.0% |
| gitdiff.txt | 22628 | 22628 | 0.0% | 20167 | 10.9% | 3223 | 85.8% |
| ls.txt | 8830 | 951 | 89.2% | 812 | 90.8% | 812 | 90.8% |
| migration.sql | 1635 | 1635 | 0.0% | 1635 | 0.0% | 1635 | 0.0% |
| npm.txt | 19345 | 11624 | 39.9% | 11288 | 41.6% | 10196 | 47.3% |
| pytest.txt | 13612 | 8802 | 35.3% | 8802 | 35.3% | 6998 | 48.6% |
| server.log | 99573 | 50835 | 48.9% | 11444 | 88.5% | 5950 | 94.0% |
| tsc.txt | 14690 | 14690 | 0.0% | 13423 | 8.6% | 4945 | 66.3% |
| **total** | **221746** | **134448** | **39.4%** | **89014** | **59.9%** | **46811** | **78.9%** |

## The ten fixes

Each row lists the fixtures whose byte count changed at that commit, measured the
same way. A fixture not listed did not move.

| # | commit | fixtures that moved, before -> after |
|---|---|---|
| 01 | `e163947` | none. The banner is stderr, so it never appears in these counts |
| 02 | `caff098` | gitdiff 22628 -> 20167, server 50835 -> 22882 |
| 03 | `df9ff96` | api 10979 -> 9139, server 22882 -> 11444, tsc 14690 -> 13423, npm 11624 -> 19345, ls 951 -> 951 |
| 04 | `c7af4b7` | pytest 8802 -> 13612 |
| 05 | `0fc5eaf` | npm 19345 -> 11288, pytest 13612 -> 8802, ls 951 -> 812 |
| 06 | `6e9592e` | cargo_test 8418 -> 283 |
| 07 | `21dd06c` | npm 11288 -> 10196, pytest 8802 -> 6998, docker 3886 -> 3630, server 11444 -> 11446 |
| 08 | `0e628b2` | gitdiff 20167 -> 3223 |
| 09 | `1bfdc74` | tsc 13423 -> 4945, server 11446 -> 5950 |
| 10 | `8347ef4` | none of the ten; a real `curl` capture went 2687 -> 250 |

Two rows look backwards because they are correctness fixes. Fix 03 stopped the
safe fallback from returning the reduction its own verifier had rejected. Before
it, `npm install` output compressed to 11580 bytes in default mode at confidence
50%, verification failed, and the failing output was returned anyway. After it,
the same rejection returns the 19345-byte input at confidence 100%, so the row
reads 11624 to 19345. Fix 04 made a single failed invariant trigger that
fallback, so `pytest.txt` went from 8802 to 13612 for the same reason. Fix 05
then made the loss legal by declaring it, and both fixtures came back smaller
than before the rejections. Returning a reduction the verifier just rejected is
the failure mode this series exists to remove.

## What each fix does

### 01 Printer banners stay out of the transcript

The CLI prints `[sqz] 123/456 tokens (73% reduction)` and its sibling status
lines on stderr, and Claude Code's Bash tool captures stderr into the tool
result, so those lines paid tokens to tell the agent about tokens. The banner is
now printed only when stderr is a terminal, with `SQZ_QUIET` still forcing it on
or off, and a run with no reduction prints no banner at all. Error and fallback
messages are never gated.

### 02 Terminal output is capped, with both ends kept

A single tool result can be hundreds of kilobytes, and the verdict of a build or
a test run is usually in its last lines. Output over 8000 tokens is cut to the
first half and the last half of that budget, with a marker naming how many
characters went missing, so the end of the run survives the cap. The split lands
on a character boundary, never inside a multi-byte character, and the budget is
counted with the same tokenizer the ratio uses.

### 03 A rejected reduction is not returned

`compress` used to re-run the pipeline in safe mode when verification failed and
then return that result without looking at it, so a rejected reduction could still
ship. A failed verification now re-runs the safe path and returns what that path
produced, and the caller never sees the output verification rejected. On `npm`
this is why the row went up: the rejection is respected, so the input comes back.

### 04 One failed invariant is enough to fall back

Confidence was an average, so a single failed `file_paths` or `json_keys` check
still scored above the fallback threshold. Every check now hard-gates the
fallback, and the average is a diagnostic only. The one exception is
`min_retention`: shrinking a 120-line log to one representative line plus a fold
marker is a 3% ratio and the correct result, so retention counts against
confidence without sending the caller back to safety.

### 05 Dropping content requires declaring it

The truncator used to drop low-entropy lines silently. It now writes
`[N low-information segments omitted]`, lists the identifiers that lived in the
dropped segments, and the verifier accepts a missing line only when the output
declares the omission. That is why `npm.txt` and `pytest.txt` fell by more than
40% without the verifier sending them back: the loss is now stated in the output
instead of being invisible to it.

### 06 Rows that differ only by an index field

`cargo test` prints `test some_name ... ok` per test. The names differ, the rest
does not, and the earlier template stage could not fold them because it treats
only timestamps, durations, and percentages as volatile, so `case_0` and `case_1`
are different lines as far as it is concerned. This
stage collapses consecutive rows with the same token count that differ in one
token, or in two tokens that share a non-digit prefix and both end in digits
(`t_7` next to `col_7`). First and last row stay verbatim, the marker names the
count and the field that varied, and a run breaks on any signal word, on two
tokens carrying `/`, `=`, or `|`, on fewer than three distinct values, and on any
buffer containing a diff header. All of those are reasons a run of rows is not
the noise it looks like.

### 07 Identity-bearing rows outrank byte-entropy filler

Entropy ranking treats a row padded with filler as low value and a long path as
high value, which is backwards: the entropy score sees distinct bytes, not use.
Identity-bearing rows are now the primary key and entropy only orders the rest,
so the drop budget is spent on filler. Two classes are never dropped at all: rows
shorter than the minimum segment length, and rows carrying an error marker.
Identity rows stay inside the budget, so a log where most rows hold a path still
compresses instead of turning into a passthrough.

### 08 Identical hunks across many files

A generated diff repeats the same hunk body in every file. The fold keeps one
exemplar verbatim and turns every other file in the group into one reference line
carrying its path and its `@@` headers, so the diff still says where every change
landed. Bodies must be byte-identical, digits included, so nothing is masked or
inferred; the group needs at least three files and at least six body lines, and
sections with mode changes, renames, copies, or binary payloads are never folded.
The fold runs before the generic stages and marks the run, which skips the
template grouping stage: a reference line next to the exemplar would look like a
repeated row and get grouped into a template, so the two stages do not stack.

### 09 One message, many payloads

`tsc` prints one error message per location, `docker` one step line per layer.
The rows share a layout and differ only in their fields, so the layout is
printed once with a `%` slot per variable field and each row's values follow.
`expand_declared_templates` rebuilds the original lines byte for byte, and the
verifier expands declared blocks before its content checks so every path,
number, and message is still required.

### 10 Progress redraws and transient runs

A progress bar is one line written many times behind `\r`, so a finished
transfer carries every frame it ever drew. The stages before this one all key on
stable text: run-length encoding needs byte-identical neighbours, the log
template masks timestamps, the indexed-row stage needs a varying index, and the
truncator scores a bar as content it must keep. A real `curl` progress meter is
therefore 0% compressed at every earlier commit in this table. The stage keeps
the last frame, which is the only one a terminal ever showed, and records how
many it replaced. Runs of consecutive transient lines (bars, transfer status,
spinner frames, "Downloading ..." lines) collapse to the first line plus a
declared count, and a line or run carrying error, warning, failure, or panic
wording is never dropped. `=====` header rules, `| ++++----` diffstat rows, and
JSON or diff payloads are refused without a percentage or bracket that marks
them as progress.

Measured on a 2687-byte capture of a rate-limited `curl` download: 2687 -> 250
bytes, the final `100  755k  100  755k ... 28583` frame intact behind
`[x32 frames]`. The same binary is byte-identical to `1bfdc74` on all ten
fixtures in the table, on a `git log --stat`, on `ls -la`, and on a 27KB test
run, so the stage costs nothing when no progress output is present.

## Safe mode refuses SQL migrations on purpose

`migration.sql` measures 0% at every commit in the table, and that is the
intended result. `confidence_router` treats a DDL line as high risk: any line
starting with `create table`, `alter table`, `drop table`, `create index`, or
`drop index` routes the whole input to `CompressionMode::Safe`, which returns the
bytes it was given. A migration is a list of statements where dropping one
changes what the file does, so no stage runs on it. Verified at HEAD: all three
modes return the 1635 input bytes unchanged, and the auto route prints
`safe (preserving all content)`. The fixture stays in the corpus as the guard
that says so: if a future change starts compressing migrations, this row will
show it.

## Verifier changes that made the reductions safe

The pipeline only gets to keep a reduction the verifier accepts. Three changes
were needed to check the new encodings without weakening the checks:

| check | what it asserts |
|---|---|
| declared omission | a missing line is allowed only when a marker line names the loss |
| template expansion | declared template blocks are expanded before content checks, then paths, numbers, messages, and hunk headers are required |
| hard gate | any failed invariant triggers the lossless fallback, not an average |

## Still open

These are measured gaps the ten fixes did not close. They need a decision about
the host or the protocol, not another pipeline stage.

| gap | why it matters | effort | risk |
|---|---|---|---|
| built-in `Read` bypasses sqz | 28.5% of sampled tokens; 61% of reads are repeats | S to M | medium |
| MCP proxy compresses only `tools/call` text | `structuredContent` and `resources/read` text are untouched | M | low |
| dedup refs are process-global | refs leak across sessions and projects | M | medium |
| Claude Code default mode skips piped commands | Bash is the largest traffic source | M | medium |
| PowerShell and OpenCode skip shell operators | partial coverage on those hosts | S to L | low |
| JSON projection removes fields with no recovery hint | a stripped `request_id` can force a second call | S | medium |
| `npx <runner>` and `deno test` miss the test formatter | unquantified | S | low |

## Evidence limits

- The corpus is ten generated fixtures that follow the real tools' line formats.
  They measure the pipeline against a known shape and keep the comparison between
  commits exact, but they are cleaner than field output: no interleaved warnings,
  no truncated lines, no mixed encodings. A fixture going from 8418 to 283 bytes
  proves the stage works on that shape, not that it finds that shape in every run,
  and nothing here measures how often safe mode or the fallback fires in the field.
- Fix 10 is the exception to the fixture caveat in both directions. Its only
  measured gain is on a real capture rather than a generated one, but that capture
  is a single transfer with no concurrent output, so it does not show how the
  stage handles a bar interleaved with build warnings, and 90% on it is one
  sample, not a distribution.
- The original 8.3M-token transcript corpus from 260920 cannot be re-run here.
  Its shares (Bash 54.3%, `Read` 28.5%, other MCP 16.3%) are the reason the open
  gaps above are ordered the way they are, but they are not current absolutes.
- Byte counts are not token counts. Stages that remove whole rows translate
  almost one to one; stages that rewrite text do not, which is why every claim
  above is in bytes from a real run.
