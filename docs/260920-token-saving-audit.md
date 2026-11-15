# sqz token-saving audit

What this document is: the fixes that came out of the 260920 audit of the
compression pipeline, what each one changed, what it measured on a fixed corpus,
and the evidence that the shipped binary does what the tables say. Numbers in
every table come from the commands shown next to them, run on this machine
against the checked-out tree and against the installed 2.2.0 binary. No number
here is a projection or a worker report.

## Where the tokens are

The only measured corpus of real traffic is 230 Claude Code transcripts: 24,364
tool results, 33,253,669 bytes, about 8.3M context tokens, collected on
2026-09-20. Those raw transcripts are gone, so the shares below are the reason
the fixes and the open gaps are ordered the way they are. They are not current
absolutes.

| source | tokens | share | calls | avg per call | sqz can reach it? |
|---|---:|---:|---:|---:|---|
| `Bash` | 4,514,780 | 54.3% | 17,247 | 261 | yes, through the hook |
| built-in `Read` | 2,370,645 | 28.5% | 1,786 | 1,327 | no, it bypasses sqz |
| other MCP servers | 1,358,809 | 16.3% | n/a | n/a | not unless proxied |
| sqz's own MCP tools | 69,139 | 0.8% | n/a | n/a | already compressed |

The 28.5% versus 0.8% split is the main coverage problem. `sqz_read_file` exists
to capture `Read` traffic and is handling almost none of it.

`Read` traffic also repeats heavily: 1,784 calls touched 692 unique paths, so 61%
were repeat reads. First reads were 1,013,453 tokens and repeat reads 1,336,753.
A repeat read that returns a `§ref:HASH§` costs about 13 tokens, so the upper
bound on recoverable tokens is about 1.32M, or 15.9% of context. Those are upper
bounds: they assume every repeat returned identical bytes and that no client
behavior limits the saving.

## The tail dominates the average

The local sqz database (`~/.sqz/sessions.db`, 476 logged compressions across 7
projects between 2026-05-09 and 2026-05-31) shows where savings were being lost.

| bucket | calls | original tokens | compressed tokens | reduction |
|---|---:|---:|---:|---:|
| all logged compressions | 476 | 3,879,007 | 3,723,802 | 4.0% |
| outputs at or below 20k tokens | 471 | 317,671 | 163,526 | 48.5% |
| outputs above 20k tokens | 5 | 3,561,336 | 3,560,276 | 0.0% |

Five calls produced 92% of the tokens and sqz removed almost none of them. The
four largest were 1,041,242, 947,983, 771,140, and 770,072 original tokens, all
from `sed` and `grep` over a 1.3 MB HTML mockup. That output was effectively a
verbatim copy. The immediate cause was segment granularity: the largest cached
original has 24 lines and only a trailing double newline, so `split_segments`
falls back to 10-line groups and produces three segments of 1,075,195, 248,877,
and 141 bytes. Median entropy is 5.998 bits, so the two large segments survive.
A 1 MB segment is never subdivided. Fix 02's cap and fix 11's JSON handling
attack this from the top. The segment splitter itself is unchanged.

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
221,746 bytes. An eleventh fixture is used only by fix 10: a 2687-byte real
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
| api.json | 26533 | 10979 | 58.6% | 9139 | 65.6% | 7267 | 72.6% |
| cargo_test.txt | 8418 | 8418 | 0.0% | 8418 | 0.0% | 283 | 96.6% |
| docker.txt | 6482 | 3886 | 40.0% | 3886 | 40.0% | 3630 | 44.0% |
| gitdiff.txt | 22628 | 22628 | 0.0% | 20167 | 10.9% | 3223 | 85.8% |
| ls.txt | 8830 | 951 | 89.2% | 812 | 90.8% | 812 | 90.8% |
| migration.sql | 1635 | 1635 | 0.0% | 1635 | 0.0% | 1635 | 0.0% |
| npm.txt | 19345 | 11624 | 39.9% | 11288 | 41.6% | 10196 | 47.3% |
| pytest.txt | 13612 | 8802 | 35.3% | 8802 | 35.3% | 6998 | 48.6% |
| server.log | 99573 | 50835 | 48.9% | 11444 | 88.5% | 5950 | 94.0% |
| tsc.txt | 14690 | 14690 | 0.0% | 13423 | 8.6% | 4945 | 66.3% |
| **total** | **221746** | **134448** | **39.4%** | **89014** | **59.9%** | **44939** | **79.7%** |

## The fixes

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
| 11 | `618cde4` | api 9139 -> 7267 |

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
counted with the same tokenizer the ratio uses. That cap is right for a terminal
transcript and wrong for a JSON body, which fix 11 covers.

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
are different lines as far as it is concerned. This stage collapses consecutive
rows with the same token count that differ in one token, or in two tokens that
share a non-digit prefix and both end in digits (`t_7` next to `col_7`). First
and last row stay verbatim, the marker names the count and the field that varied,
and a run breaks on any signal word, on two tokens carrying `/`, `=`, or `|`, on
fewer than three distinct values, and on any buffer containing a diff header.
All of those are reasons a run of rows is not the noise it looks like.

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

### 11 The cap shrinks arrays instead of slicing machine-readable text

Fix 02's cap assumed the payload was a transcript. On a probe API response it
held the first and last half of a TOON body, which left text no TOON decoder or
JSON parser accepts, and the tail it kept had already lost the summary element
that said how many records went missing: 9139 bytes that decode to nothing and
do not report their own loss. JSON now answers the cap by dropping whole array
elements and re-encoding, and any count an earlier stage declared is read back
out and folded into the new summary, so kept plus declared still equals the
original length. Terminal output keeps the head and tail cap.

| commit | api.json | orders shown | dropped count declared |
|---|---:|---|---|
| `1bfdc74` | 9139 | 98 of 120 | no, the summary was inside the cut |
| `618cde4` | 7267 | 56 of 120 | yes, `... and 64 more items` |

The row costs 1872 fewer bytes and is the only fixture where the smaller output
is also the one that reports what it dropped. Dropping the trailing summary
element breaks the TOON table form, so the kept records print as objects rather
than as table rows; that is a byte cost this table already counts. Showing fewer
records than `1bfdc74` is the price of fitting the declared count inside the
budget: 56 records plus a count of 64 is recoverable, 98 records with the count
cut away is not.

## Evidence the fixes are working

Everything below was run against `sqz 2.2.0` built from this tree and installed
at `~/.cargo/bin/sqz`, with `/usr/local/bin/sqz` symlinked to it, so it measures
the shipped binary and not a dev-only path. Every command is copy-pasteable.

### The installed binary reproduces the table

```bash
for f in /tmp/sqzcorpus/{api.json,cargo_test.txt,docker.txt,gitdiff.txt,ls.txt,
         migration.sql,npm.txt,pytest.txt,server.log,tsc.txt,progress.txt}; do
  printf '%s dev=%s installed=%s\n' "$(basename $f)" \
    "$(SQZ_NO_DEDUP=1 ./target/debug/sqz compress --no-cache < $f | wc -c)" \
    "$(SQZ_NO_DEDUP=1 sqz compress --no-cache < $f | wc -c)"
done
```

| fixture | dev tree | installed | table value |
|---|---:|---:|---:|
| api.json | 7267 | 7267 | 7267 |
| cargo_test.txt | 283 | 283 | 283 |
| docker.txt | 3630 | 3630 | 3630 |
| gitdiff.txt | 3223 | 3223 | 3223 |
| ls.txt | 812 | 812 | 812 |
| migration.sql | 1635 | 1635 | 1635 |
| npm.txt | 10196 | 10196 | 10196 |
| pytest.txt | 6998 | 6998 | 6998 |
| server.log | 5950 | 5950 | 5950 |
| tsc.txt | 4945 | 4945 | 4945 |
| progress.txt | 250 | 250 | 250 |

Zero disagreements. The eleven outputs sum to 45,189 bytes: 44,939 for the ten
fixtures plus 250 for the capture. The ten-fixture total is 79.7% below the
221,746-byte input and 66.6% below the 89,014 the same corpus measured at
`fix05`.

### Nothing was dropped without saying so

The reduction is only worth having if the model can tell what is missing. For
each fixture this compares the distinct indexed identifiers in the input with the
identifiers in the output, and counts declaration markers in the output.

```bash
out=$(SQZ_NO_DEDUP=1 sqz compress --no-cache < "$f")
grep -oE '[A-Za-z_][A-Za-z_0-9]*_[0-9]+' "$f" | sort -u | wc -l
printf '%s' "$out" | grep -oE '[A-Za-z_][A-Za-z_0-9]*_[0-9]+' | sort -u | wc -l
printf '%s' "$out" | grep -cE 'omitted|more items|frames\]|template ×|chars omitted'
```

| fixture | ids in | ids out | declaration markers | verdict |
|---|---:|---:|---:|---|
| api.json | 121 | 56 | 1 (`... and 64 more items`) | declares the 64 it dropped |
| cargo_test.txt | 300 | 2 | 1 (`field 2 spans ... case_299`) | declares 298 rows |
| docker.txt | 0 | 0 | 2 | template block plus count |
| gitdiff.txt | 46 | 46 | 0 | nothing dropped, every path kept |
| ls.txt | 60 | 39 | 1 | declares the omission |
| migration.sql | 80 | 80 | 0 | safe mode, byte-identical |
| npm.txt | 0 | 0 | 1 (`20 low-information segments omitted`) | declares |
| pytest.txt | 320 | 170 | 1 (`15 low-information segments omitted`) | declares |
| server.log | 0 | 0 | 16 | cap marker plus segment counts |
| tsc.txt | 0 | 0 | 3 (`[template ×67]`) | layout declared once |
| progress.txt | 0 | 0 | 1 (`[x32 frames]`) | final frame kept |

No fixture lost identifiers without a marker, and the two fixtures that drop
nothing (`gitdiff.txt`, `migration.sql`) keep every identifier they had.

### The verifier accepted all of it, and safe mode still refuses risk

```bash
for f in /tmp/sqzcorpus/*; do
  SQZ_QUIET=1 SQZ_NO_DEDUP=1 sqz compress --no-cache < "$f" 2>&1 >/dev/null \
    | grep -c fallback
done | paste -sd,
```

Run over all eleven fixtures: `0,0,0,0,0,0,0,0,0,0,0`. No fixture reached the
lossless fallback, so each reduction above is one the verifier accepted on its
own terms rather than a rejection that was quietly dropped.

```bash
SQZ_NO_DEDUP=1 sqz compress --no-cache --lossless < /tmp/sqzcorpus/api.json | wc -c
SQZ_NO_DEDUP=1 sqz compress --no-cache --lossless < /tmp/sqzcorpus/server.log | wc -c
```

`--lossless` returns 26533 and 99573 bytes, exactly the inputs, so the gate the
fixes depend on still holds. `migration.sql` returns its 1635 input bytes under
`auto`, `safe`, and `aggressive`, and `--mode safe` prints
`[sqz] mode: safe (preserving all content)` because the router agrees the input
is DDL.

### Per-fix behaviour, not just byte counts

| fix | command | observed |
|---|---|---|
| 01 | `sqz hook claude < hook.json 2>err` | valid JSON on stdout, `err` is 0 bytes, no `[sqz]` banner |
| 02 | `compress < server.log \| grep chars` | `[... 38653 chars omitted: output exceeded the 4000-token cap; kept first and last halves ...]` |
| 03 | `compress --no-cache < npm.txt 2>&1 >/dev/null` | no fallback line, so the shipped output is one verification accepted |
| 04 | `cargo test --workspace` | 1807 passed, 0 failed across 15 test binaries, including the invariant and fallback suites |
| 05 | `compress < npm.txt \| grep omitted` | `[20 low-information segments omitted]`; pytest prints `[15 low-information segments omitted]` |
| 06 | `compress < cargo_test.txt \| head -6` | `test tests::case_0 ... ok`, `[×298 rows omitted: field 2 spans "tests::case_0" .. "tests::case_299", all other fields identical]`, `test tests::case_299 ... ok` |
| 07 | `compress < server.log \| grep -ci error` | 1 in, 1 out: the only error line survives; the 60 filler segments behind it are the declared loss |
| 08 | `diff <(grep -o 'src/mod_[0-9]*\.rs' gitdiff.txt \| sort -u) <(compress < gitdiff.txt \| grep -o ... \| sort -u)` | no output: all 40 paths survive, 1 exemplar plus 39 reference lines |
| 09 | `compress < tsc.txt \| head -8` | `[template ×67]` followed by the 67 paths with their `(line,col)` values |
| 10 | `compress < progress.txt \| tail -3` | final `100  755k  100  755k ... 28583` frame kept, `[x32 frames]` appended |
| 11 | `compress < api.json \| tail -c 40` | `... and 64 more items` inside a syntactically closed TOON body, so the count survives the cap |

Fix 09's round trip is covered by tests rather than by eye: four tests in
`sqz_engine/src/template_collapse.rs` assert
`expand_declared_templates(&folded.text) == input`, and the verifier calls the
same expansion before checking paths, numbers, and messages.

### Real command output, not only fixtures

Same binary, same flags, three outputs produced by real commands on this machine.

| capture | input bytes | `fix05` | HEAD | notes |
|---|---:|---:|---:|---|
| `git log -40 --stat` | 31852 | 21358 | 20174 | all 2 distinct commit shas retained |
| `ls -la` | 3299 | 3299 | 3040 | passthrough before, compressed now |
| `git diff --stat HEAD~1 HEAD` | 188 | 188 | 188 | no repeated hunk body, so the fold correctly does not fire |

The third row is the control: a shape the new stages have nothing to say about
comes back at exactly the same size, which is what "costs nothing when the shape
is absent" has to mean.

The pressure path is also no longer a no-op. Under an explicit mode,
`server.log` measures 11878 bytes with `--mode default` and 5906 with
`--mode aggressive`, so the escalation branch now reaches a different preset
instead of returning to the same call. Note that explicit-mode numbers differ
from the stdin table above, which runs the hook path with no `--mode`.

### The failure this series started from is gone

The audit began with a 1.3 MB `sed`/`grep` result that compressed to a verbatim
copy, 105 bytes removed from 1,324,216. That input is no longer available, but
the mechanism it exposed is measured above: output over the cap is now cut with a
declared character count, JSON is shrunk by whole array elements with the count
of what went missing, and no repair path returns a reduction its own verifier
rejected. The largest fixture in the corpus, a 99,573-byte server log, goes to
5950 bytes with 16 declarations and zero fallbacks.

## Safe mode refuses SQL migrations on purpose

`migration.sql` measures 0% at every commit in the table, and that is the
intended result. `confidence_router` treats a DDL line as high risk: any line
starting with `create table`, `alter table`, `drop table`, `create index`, or
`drop index` routes the whole input to `CompressionMode::Safe`, which returns the
bytes it was given. A migration is a list of statements where dropping one
changes what the file does, so no stage runs on it. Verified at HEAD: all three
modes return the 1635 input bytes unchanged, and `--mode safe` prints
`safe (preserving all content)`. Auto mode prints nothing because the banner is
tty-gated, which is fix 01 doing its job. The fixture stays in the corpus as the
guard that says so: if a future change starts compressing migrations, this row
will show it.

## Verifier changes that made the reductions safe

The pipeline only gets to keep a reduction the verifier accepts. Three changes
were needed to check the new encodings without weakening the checks:

| check | what it asserts |
|---|---|
| declared omission | a missing line is allowed only when a marker line names the loss |
| template expansion | declared template blocks are expanded before content checks, then paths, numbers, messages, and hunk headers are required |
| hard gate | any failed invariant triggers the lossless fallback, not an average |

## Still open

These are measured gaps the fixes did not close. They need a decision about
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
| segment splitter still cannot subdivide a 1 MB segment | the tail case from the original audit is bounded by the cap, not fixed at the source | M | low |

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
- The three real captures in the evidence section are three commands from one
  repository on one machine. They show the stages do not cost bytes where they
  have nothing to fold, and they show a real gain, but they are not a
  distribution either.
- The original 8.3M-token transcript corpus from 260920 cannot be re-run here.
  Its shares (Bash 54.3%, `Read` 28.5%, other MCP 16.3%) are the reason the open
  gaps above are ordered the way they are, but they are not current absolutes.
- Byte counts are not token counts. Stages that remove whole rows translate
  almost one to one; stages that rewrite text do not, which is why every claim
  above is in bytes from a real run.
- The evidence section proves the binary reproduces the table and that each
  declaration is present. It does not prove a model saves tokens on every run:
  that needs the hook installed in a live session and the `sqz gain` counters,
  which measure real traffic, not this corpus.
