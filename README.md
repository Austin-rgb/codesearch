# codesearch

Find Rust functions by their **parameter and return types** instead of their names.

```console
$ codesearch "(Username, Password) -> Session"

Found 2 matching functions:

1. login
   Signature: (Username, Password) -> Session
   File: src/auth/login.rs
   Line: 4
   Module: auth::login

2. authenticate
   Signature: (Username, Password) -> Session
   File: src/auth/service.rs
   Line: 4
   Module: auth::service

2 matches found.
```

It parses your `.rs` files with [`syn`](https://crates.io/crates/syn), stores the signatures in a
project-local SQLite database, and answers queries with a structural (not textual) type match.
Everything runs offline; there are no background services.

## Install

Requires a recent stable Rust toolchain and a C compiler (SQLite is compiled into the binary, so
there is no runtime dependency on a system SQLite).

```sh
cargo install --path .        # installs `codesearch` into ~/.cargo/bin
# or
cargo build --release         # binary at target/release/codesearch
```

Run the tests with `cargo test`.

## Usage

```sh
codesearch "(Username, Password) -> Session"             # search the current project
codesearch --project ./my-project "UserId -> User"       # search another project
codesearch index                                         # build / refresh the index of .
codesearch index ./my-project                            # ... of a specific directory
codesearch stats                                         # show index statistics
```

The index is created automatically on first use, and every search first refreshes it
incrementally (a quick stat of each file; only changed files are re-read). Pass `--no-update` to
skip that check, or run `codesearch index --force` to re-hash every file regardless of
modification time.

A query that doesn't parse is rejected before the index is touched:

```console
$ codesearch "Username, Password -> Session"
error: invalid query: multiple parameters must be wrapped in parentheses
  Username, Password -> Session
          ^
  hint: write "(A, B) -> C" instead of "A, B -> C"
```

Exit status is `0` on success (including "no matches") and `2` on errors.

### Options

| Option | Applies to | Effect |
| --- | --- | --- |
| `-p, --project <DIR>` | all | Project directory (default `.`). The index lives in `<DIR>/.codesearch/`. |
| `--ignore-self` | search | Skip a `self` receiver when comparing parameters (see below). |
| `--no-update` | search | Don't refresh the index before searching. |
| `--force` | `index` | Re-hash every file even if size and mtime are unchanged. |

### Try it

```sh
codesearch --project examples/sample-project "(Username, *) -> Session"
codesearch --project examples/sample-project --ignore-self "UserId -> Option<UserDetails>"
```

## Query language

```
(ParamType, ParamType, ...) -> ReturnType
```

| Query | Meaning |
| --- | --- |
| `UserId -> UserDetails` | one parameter (parentheses optional) |
| `(Username, Password) -> Session` | several parameters; **order and count matter** |
| `() -> Config` | no parameters |
| `UserId -> ()` | returns unit (also matches functions with no `->`) |
| `(UserId, TenantId) -> (User, Metadata)` | tuple return |
| `UserId -> Option<UserDetails>` | generics, nested to any depth |
| `(Username, *) -> Session` | `*` matches exactly one type of any shape |
| `(Username, Password) -> *` | wildcard return |
| `(&str, &mut Vec<u8>) -> &str` | references (lifetimes are accepted and ignored) |
| `&[T] -> T`, `[u8; 4] -> u32`, `[u8; *] -> u32` | slices and arrays |
| `impl Display -> Box<dyn Error>` | `impl` / `dyn` trait types |

Notes:

* A query starting with `(` is always a parameter list: `(A, B) -> C` has two parameters,
  `((A, B)) -> C` has one tuple parameter.
* Named types match by path suffix: `Session` matches `auth::Session`, and `collections::HashMap<K, V>`
  matches `std::collections::HashMap<K, V>` as well as a bare `HashMap<K, V>`.
* Generic argument counts must agree: `Result<T>` does not match `Result<T, E>`. Use `Vec<*>` to
  ignore the argument.
* `Foo<Item = T>` associated-type bindings and `Fn(A) -> B` sugar can't be written in a query yet
  (use a wildcard for those parameters).

### Methods and `self`

A `self` receiver is the first parameter, typed `Self`, `&Self` or `&mut Self`, so:

```sh
codesearch "(&Self, UserId) -> User"        # fn get(&self, id: UserId) -> User
codesearch --ignore-self "UserId -> User"   # the same function, receiver hidden
```

Without `--ignore-self`, `UserId -> User` does **not** match that method.

## How it works

```
src/
├── types.rs      TypeExpr AST, Query, FunctionSig (shared by every stage)
├── parser.rs     query string  → Query            (hand-written lexer + recursive descent)
├── extractor.rs  Rust source   → Vec<FunctionSig> (syn visitor)
├── matcher.rs    Query × FunctionSig → bool       (recursive structural match)
├── index.rs      SQLite schema, incremental update, candidate streaming
├── search.rs     orchestration + terminal formatting
├── lint/         `codesearch lint`: Rule trait, config, rules/r001.rs, rules/r002.rs, report.rs
├── cli.rs        clap definitions
├── lib.rs        module wiring (needed so tests/ can import the crate)
└── main.rs       thin binary entry point
```

**Extraction** records name, parameter names and types, return type, file, line, module path
(from the file path plus inline `mod`s), whether it is a function or an `impl`/`trait` method, and
generic parameter names. Trait declarations and trait impls are indexed too.

**Indexing** keeps one row per file (size, mtime, BLAKE3 hash) and one per function. On update:
unchanged size+mtime → skipped without reading; changed mtime but same hash → metadata refreshed
only; different hash → re-parsed and replaced; vanished files → deleted. Types are stored as JSON
plus two plain columns, `param_count` and `ret_head` (the last path segment of the return type),
with an SQLite index on both.

**Searching** asks SQLite only for rows with the right parameter count and (unless the query's
return type is `*`) return-type head, streams them one at a time, and runs the full structural
matcher on each. Memory use therefore scales with the number of *matches*, not the size of the
codebase; the index scales with the project, not the filesystem.

`.git`, `target`, `node_modules` and `.codesearch` are never scanned. The index directory
contains its own `.gitignore`, so it won't be committed.

Files that `syn` can't parse (e.g. mid-edit syntax errors) are recorded with zero functions,
counted in `codesearch stats`, and retried as soon as their content changes.

## Lint

`codesearch lint [PATH]` flags **public functions that are hard to find or easy to misuse** through
signature search. It reads the same SQLite index as search (refreshed incrementally first, so a
second run re-parses nothing that did not change).

```sh
codesearch lint                                    # current project, text output
codesearch lint ./my-project --format json
codesearch lint --rule R001 --fail-on warn         # one rule; exit 1 if anything is flagged
```

### Rules

| Id | Name | Flags a `pub` function when... |
| --- | --- | --- |
| `R001` | primitive-heavy-signature | **(a)** two or more parameters share the same bare primitive type; **(b)** a parameter (or the return type) is a bare primitive and the parameter name (or function name) contains a domain word such as `id`, `email`, `amount`; **(c)** it has three or more parameters and all are bare primitives |
| `R002` | missing-intent-line | **(a)** it has no doc comment; **(b)** the first doc line is empty, shorter than 15 characters, or only restates the function name; **(c)** the first doc line does not end with a period, or the first paragraph exceeds 200 characters |

Both rules apply only to plain `pub` functions and methods (not `pub(crate)`, `pub(super)`,
`pub(in ..)` or private ones). A function gets at most one finding per rule, listing every
clause it violated.

How the rule text was interpreted:

* *Bare primitive*: `String`, `str`, `bool`, `char`, integers, floats, `usize`, `isize`, also
  behind references (`&str`). Primitives inside generics (`Vec<u8>`, `Option<String>`) count only
  for R001(b). "Same type" in (a) is the type as written, so `String` and `&str` differ.
* A `self` receiver is not counted as a parameter by R001.
* Domain words match whole `_`-separated name tokens (and a plural `s`): `user_id` and `emails`
  match, `paid`, `monkey` and `identity` do not.
* Methods in trait impls are skipped by R001 (their signature belongs to the trait).
* Lengths count characters; exactly 15 (and exactly 200) pass.
* The default domain words are `id, uid, email, name, username, password, token, key, url, uri,
  path, user, tenant, session, amount, currency, timestamp`.

### Output

Text output groups findings by file, one `file:line  RULE  message` line each followed by a
suggestion, then a summary:

```text
$ codesearch lint examples/sample-project
src/billing.rs:2  R001  primitive-heavy signature in `charge`: parameters `user_id` and `amount` share ...
    suggestion: Consider a newtype such as `UserId` for parameter `user_id: u64`. ...

Summary
  Public functions:   21
  Flagged functions:  14
  Findings:           14 (R001: 3, R002: 11)
  Discoverability:    0.33
```

`--format json` prints `{"findings": [...], "summary": {...}}`. Each finding has `rule_id`,
`severity` (`warn`), `file`, `line`, `module`, `function`, `message` and `suggestion`. The summary has
`public_functions`, `flagged_functions`, `total_findings`, `findings_by_rule` (every rule that ran,
including zeros) and `discoverability_score`, which is
`1 - flagged_functions / public_functions` (`1.0` when there are no public functions).

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | No findings, or `--fail-on never` (the default) |
| `1` | Findings exist and `--fail-on warn` was given |
| `2` | An error (bad path, unknown rule, broken config, bad arguments) |

### Configuration

Everything is optional; with no file the defaults above apply. Create `.codesearch/lint.toml`
(the index's own `.gitignore` leaves that one file committable):

```toml
enabled_rules  = ["R001", "R002"]        # default: every rule
domain_words   = ["id", "email", "sku"]  # replaces the default list
min_intent_len = 15                      # R002: minimum first-doc-line length
ignore         = ["tests/**", "**/generated/**", "*_test.rs"]   # files left out entirely
```

Ignore globs are matched against `/`-separated paths relative to the project: `*` stays inside
one path segment, `**` crosses directories, and a pattern without `/` matches the file name only.
Ignored files do not count towards `public_functions`. `--rule` on the command line overrides
`enabled_rules`. Unknown keys in the file are an error, so typos are caught.

### Opting out

A plain `//` comment directly above a function (above or between its attributes and doc
comment) silences the named rules for that function:

```rust
// codesearch-lint: allow(R001)
// codesearch-lint: allow(R001, R002)
```

The function still counts as public in the score; it just cannot be flagged by those rules.
Opt-outs are read during extraction and stored in the index.

### Adding a rule

Implement `lint::Rule` (`id()` and a pure `check(&FunctionSig, &LintConfig) -> Vec<Finding>`) in a
new `src/lint/rules/` module and add one line to `rules::all()`. Selection, opt-outs, ignores,
reporting and exit codes all work on rule ids, so none of that code changes.

## Scope and limitations

This is a prototype of *syntactic* search. By design it does not do:

* compiler-based type resolution or inference,
* type alias resolution (`type Res<T> = Result<T, MyError>` won't match `Result<T, MyError>`),
* trait implementation discovery,
* macro expansion (functions generated by macros are invisible),
* other languages, or IDE integration.

Also worth knowing: `cfg` attributes are ignored (every cfg branch is indexed), `where` clauses and
lifetimes are dropped, generic parameters match by the name written (`T -> T`), and the matcher
compares `dyn A + B` bounds in order.

### Extending it

The seams for the features above are already in place:

* **Alias / `Self` / trait-aware matching**: implement `matcher::TypeResolver` and pass it to
  `Matcher::new`. Every candidate type goes through it before comparison, and neither the query
  parser nor `type_matches` needs to change. (A resolver that alters a type's outermost name
  shouldn't be combined with the `ret_head` SQL pre-filter.)
* **Other languages**: write an extractor that emits `FunctionSig` values with `TypeExpr` types;
  the parser, matcher and index are language-agnostic. Bump `SCHEMA_VERSION` in `index.rs` if the
  stored shape changes — a mismatch rebuilds the index automatically.

## Caveats

* Change detection trusts size + mtime before hashing. An edit that preserves both (rare) is
  picked up by `codesearch index --force`.
* The index is per directory you point `--project` at; running from a subdirectory builds a
  separate index there.
