# Revision management

Two kinds of storage keep versions of a file behind the same resource API: `dbresource://` files, whose past versions
are `DbResourceFileHistory` rows, and `git://` files, whose versions are commits. Both answer the version methods of
`ResourceReference`, a line diff between any two versions, and `location#versionName` reads. The wiki services
(`org.moqui.impl.WikiServices`) use only that API, so a wiki space works on either storage.

## The version API

From a script, on `ec.resource.getLocationReference(location)`:

| Method | Answer |
|---|---|
| `supportsVersion()` | True for a `dbresource://` or `git://` file |
| `getVersionHistory()` | Every version, newest first: `{versionName, previousVersionName, versionDate, userId}` with Java's `getVersionName()` and the other getters, `getText()`, `getPreviousVersion()`, `getNextVersions()` and `getMap()` |
| `getCurrentVersion()`, `getRootVersion()`, `getVersion(name)`, `getNextVersions(name)` | As in Java |
| `getText(versionName)` | The text of that version, or null |
| `getVersionDiff(lhs, rhs)` | The line diff from version `lhs` to `rhs` (an empty name is the current text): `{fromName, toName, unifiedDiff, html, stats: {insertions, deletions, hunks}, hunks}`. A version that is not there throws |

`ec.resource.getLocationText("dbresource://wiki/page.md#02")`, and a `render-mode` `text location` with a `#version`
suffix, read that version, as Java's `getLocationText` does. A suffix on storage that keeps no versions is ignored.

`html` is a `<table class="moqui-diff">`: old and new line numbers, a sign column, the changed words of a changed line
in `<del>` and `<ins>`, and every text escaped (with `{` written as `&#123;`, so a qvt screen's Vue compile reads no
interpolation in it). The `moqui-diff` rules in the webroot CSS style it in light and dark. Put it in a screen with
`<render-mode><text type="html,qvt"><![CDATA[${diff.html}]]></text></render-mode>`; the gallery's Editor screen
(`runtime/base-component/example/screen/Gallery/Editor.xml`) and SimpleScreens' `WikiCompare` do. The diff is
Patience over lines, with three lines of context ([`similar`](https://crates.io/crates/similar)); Rust code calls
`ResourceFacade::version_diff` or `crate::resource::textdiff`.

## `dbresource://` history

Each write of a file that already exists moves the text it replaces into that version's `DbResourceFileHistory` row
and adds the next version (`01`, `02`, ...), as in Java. What the replaced row holds:

- **A reverse delta** (`isDiff="Y"`): a unified diff from the next version's text back to this one (`--- 03` /
  `+++ 02`). `DbResourceFile` always holds the newest text whole.
- **The whole bytes** (`isDiff="N"`) when the file's MIME type is binary, either text is not UTF-8, the version
  number is a multiple of 20, or the delta would be more than half the size of the text it replaces.

Reading a delta version starts from the nearest whole text after it (the file, or a whole past row) and applies the
deltas down to it, so a read applies at most 19 of them. A delta that does not apply exactly (no fuzz) is logged with
the location and version, and that version reads as not there; the other versions still read.

Java writes every history row whole and reads an `isDiff="Y"` row as null, so a Java server sharing the database sees
the delta versions as missing (the current text is unaffected). Rows loaded from data files with `isDiff="N"` read as
before.

## `git://` locations

`git://<repositoryId>/<path>` is the file at `path` on the branch of a git repository. `git://<repositoryId>/` is the
top directory. A write is a commit made at once on the branch, by the current user (`UserAccount.userFullName` and
`emailAddress`), with a `Moqui-User-Id:` trailer the history reads back as `userId`. A move is one commit; a delete of
a directory that still has files is refused, as for the other schemes. Git keeps no empty directories, so a new
directory gets an empty `.gitkeep`, and a directory goes away with its last file.

A file's versions are the commits on the branch's first-parent line whose copy of the file differs from their parent's;
a commit that deletes the file is not one. Version names are full commit ids; a unique prefix of 7 or more digits
also works. The history is walked from the branch head on each call, which suits wiki-sized repositories.

### Repositories

Each repository is a `moqui.resource.GitRepository` row, edited under System, Resource Finder, Git Repositories:

| Field | |
|---|---|
| `repositoryId` | The first segment of the location |
| `repositoryKind` | `db`, `local`, or `remote` (below) |
| `branch` | The branch read and written; `main` when empty |
| `localPath` | The bare repository of a `local` or `remote` kind; relative to the runtime directory unless absolute; `runtime/git/<repositoryId>` when empty |
| `remoteUrl` | Fetched from by `remote`; the default source of `import#GitRepository` |
| `username`, `accessToken` | HTTPS credentials for a fetch; the token is an encrypted field, and the screen leaves it unchanged when its field is empty |
| `fetchIntervalMinutes` | For `remote`: how often the fetch job fetches it; never when empty |
| `lastFetchDate`, `lastFetchError`, `headCommitId` | Written by fetches and commits |

- **`db`**: objects and refs are `GitObject` and `GitRef` rows (raw object content, no git header, uncompressed;
  Postgres compresses large `bytea` values). The database backup holds the repository. Objects are byte for byte
  git's, so an export has the same commit ids. Two servers writing the same branch are kept consistent by the ref
  update, which locks the `GitRef` row and compares before it writes.
- **`local`**: a bare repository on disk, created on first use (or by `init#GitRepository`). Clone it, or push to it,
  with the git command line; a push to its branch is seen on the next read.
- **`remote`**: a read-only mirror of `remoteUrl`, under `localPath`. Writes are refused. `fetch#GitRepository`
  replaces the mirror's branch with the remote's; the first fetch creates the mirror. A fetch is refused in sim mode.

### Services

`org.moqui.impl.GitServices` (`runtime/base-component/framework/service/org/moqui/impl/GitServices.xml`). A failure is
an error in the message facade, as for any service.

| Service | |
|---|---|
| `fetch#GitRepository(repositoryId)` | Fetch a `remote` repository now |
| `fetch#AllGitRepositories` | Fetch each `remote` repository whose `fetchIntervalMinutes` has passed; a failure is recorded on that row and logged, and the others are still fetched. The `fetch_AllGitRepositories_frequent` ServiceJob runs it every 5 minutes |
| `init#GitRepository(repositoryId)` | Create a `local` bare repository; fetch a `remote` one; nothing for `db` |
| `export#GitRepository(repositoryId, targetPath)` | Write a `db` repository as a bare repository on disk, with its branch and `HEAD` |
| `import#GitRepository(repositoryId, source)` | Copy a branch with its history into a `db` repository, replacing its branch. `source` is `git://otherRepositoryId`, a repository directory (bare or not), or a URL; the row's `remoteUrl` when empty |

### Wiki spaces on git

Create a space with its root under a repository, for example
`create#WikiSpace(wikiSpaceId: "DOCS", rootPageDirectory: "git://docs")`, or point an existing `WikiSpace.rootPageLocation`
at `git://<repositoryId>/<SPACE>.md`. Each page save is then a commit, and the page history, compare, publish and
`#version` views work on commits. The gallery's Editor screen makes such a space (`EXGIT`, in the `db` repository
`example-wiki`) with its "Create the git space" action.

### Limits

- **No push.** gitoxide, the git library, does not push yet. To publish a `db` repository, export it and push the
  export with the git command line; a `local` repository can be pushed from directly.
- **Transports.** HTTPS is built in (reqwest on rustls). A `file://` or local-path URL in a fetch runs the system
  `git-upload-pack`, and an `ssh://` URL the system `ssh`; import from a local repository directory or another
  `git://` repository needs neither, as it copies objects directly.
- SHA-1 repositories only. No submodules, LFS, or signed commits.
- The `git` cargo feature of `moqui-core` (on by default, off in the browser build) links gitoxide. Without it the
  `git` scheme is not installed.
