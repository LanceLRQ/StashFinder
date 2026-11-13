# Search and duplicate candidates

[简体中文](search.md) | English

Search reads registered partitions' local indexes, including online and offline records. Historical registrations require an explicit filter. Results do not imply that source files can be opened. Search neither reads contents nor modifies source files.

## Expressions

Press F1 or choose Help → Search syntax to see query examples offline in the app.

| Input | Meaning |
|---|---|
| `aaa bbb` | Both substrings, in any order, across path components by default |
| `"aaa bbb"` | Contiguous phrase |
| `a\|b c` | `(a OR b) AND c`; OR binds before AND |
| `<a b>\|c` | Explicit grouping |
| `!backup` | Exclude matching records |
| `*.mkv`, `photo-?.jpg` | Wildcards without separators target names; explicit paths target relative paths |
| `path:projects`, `name:report` | Force path or name matching |
| `parent:"projects\demo"` | Direct children of the specified parent |
| `ancestor:projects` | Children and descendants, with directory boundaries |
| `file:`, `folder:` | Entry type |
| `ext:mkv;mp4` | Extensions combined with OR |
| `size:>=1GiB`, `size:0..1MiB` | Size comparison or inclusive range; unknown sizes and folders do not qualify |
| `dm:today`, `dc:2026-11-01..2026-11-05` | Modification/creation dates using local day boundaries |
| `regex:"(?=report).*\.pdf$"` | Local PCRE2 condition; quote query syntax characters |

`C:\aaa` uses current online mounts only. Select offline partitions and use relative paths instead of historical drive letters. Ordinary matching normalizes `/` and `\`, and defaults to Unicode lowercase comparison. Whole-word mode uses PCRE2 Unicode word boundaries.

Sizes support B, KB/MB/GB/TB (powers of 1000), KiB/MiB/GiB/TiB (powers of 1024), and up to six decimal places. Comparisons are `= > >= < <=`. Dates support `today`, `yesterday`, `thisweek` (Monday start), and `thismonth`; unknown timestamps do not satisfy date comparisons.

Filters provide case sensitivity, whole name, whole word, name prefix/suffix, wildcard and regex modes. Quote whole names containing spaces. Global regex treats the entire input as a PCRE2 pattern while graphical filters still apply; local `regex:` combines with surrounding expressions.

PCRE2 uses UTF/UCP with per-match limits of 100,000 match steps, depth 128, and a 4 MiB heap budget. Invalid expressions, unsupported functions, resource limits, invalid ranges and excessive nesting produce errors, not empty success. Input is limited to 4096 bytes and grouping/negation to 24 levels. This is the documented compatibility subset, not every Everything function.

## Types and scope

Selected types combine with OR, then AND with other conditions. Settings → File categories can add, edit and remove extension categories. Leading dots and case are normalized; names must be unique, and extensions cannot contain paths or wildcards. Categories are stored in the current user's configuration without changing system associations.

Search → Filter presets and the search toolbar's Filter dialog contain built-in file types and personal presets. Save file or folder kind, extensions, size and creation or modification dates as a preset, or maintain them from Search → Manage filter presets. Applying a preset in a browse tab filters the right-hand list and keeps the directory tree available. The app remembers that browse choice and uses it as the default for new searches. Each search tab can override or clear it. Closing Advanced search discards unapplied search conditions.

In the active search tab, Ctrl+I toggles case sensitivity, Ctrl+B whole-word matching, Ctrl+U path matching, and Ctrl+R regular expressions. Whole-word matching and regular expressions share one match mode.

Press Ctrl+D or choose Bookmarks → Add current search to save its text, scope, matching options, file filters and sort order. Opening a bookmark creates a search tab. Bookmarks scoped to a partition or folder can search the index while the partition is offline. If the saved filesystem or folder is unavailable, choose a new scope.

A search started from browsing can target its original partition or folder and descendants. Individual partitions, online-only and historical-record filters are also available. Folder scope uses indexed directory IDs; missing directories produce an error instead of a same-name fallback.

## Duplicate candidates

| Expression / graphical choice | Grouping key |
|---|---|
| `dupe:`, `dupe:name` / Same name | Complete name, separating files and folders |
| `dupesize:`, `sizedupe:`, `dupe:size` / Same size | Exact byte size, files only |
| `dupe:name;size` / Same name and size | Joint name and size |
| `dupe:dm` / Same modification time | Exact stored FILETIME |
| `dupe:name;size;dm` | All three attributes together |

Ordinary partition, state, type, text, size and date filters run first. Grouping covers all matching records, followed by global sorting and pagination. Multiple duplicate conditions form one joint key and cannot appear inside OR, negation or groups. Missing key fields are excluded and counted; real zero-byte files qualify.

A candidate group needs two distinct file objects. Multiple hard links alone do not establish another object. Qualifying groups retain all matching paths and label hard links. Offline records are snapshots. Matching metadata establishes neither identical contents nor recoverable space. Hashing, automatic deletion and file operations are outside this stage.

## Large results

After a search completes, choose File → Export search results or press Ctrl+S to save the selection or all matches as UTF-8 CSV. Export preserves the result order and includes unloaded rows and offline records. Paths are volume-relative, times are UTC, and unknown fields are blank.

Column headers and filters sort the complete result set; unknown numeric values remain last. The UI loads a bounded viewport window instead of creating millions of DOM rows. Ctrl/Shift selection crosses pages, and Ctrl+A selects the entire query result. Context actions can browse a result's parent in a new tab.

Changing the query, applying new conditions or closing a search tab cancels the replaced query and ignores its old responses. The backend retains index snapshots so updates do not reorder an existing query's pages. Unavailable partitions are listed while others remain searchable; query failure and an empty result are distinct.

References: [Everything searching](https://www.voidtools.com/support/everything/searching/), [PCRE2 patterns](https://www.pcre.org/current/doc/html/pcre2pattern.html). Measurements are in the [query baseline](query-performance.en.md).
