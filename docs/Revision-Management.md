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
| `getVersionHistory()` | Every version, newest first: `{versionName, previousVersionName, versionDate, userId, note}` with Java's `getVersionName()` and the other getters (`getNote()`), `getText()`, `getPreviousVersion()`, `getNextVersions()` and `getMap()` |
| `getCurrentVersion()`, `getRootVersion()`, `getVersion(name)`, `getNextVersions(name)` | As in Java |
| `getText(versionName)` | The text of that version, or null |
| `getVersionDiff(lhs, rhs)` | The line diff from version `lhs` to `rhs` (an empty name is the current text): `{fromName, toName, unifiedDiff, html, stats: {insertions, deletions, hunks}, hunks}`. A version that is not there throws |

A version's `note` is what was written with it (below, "Version notes").

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
a commit that deletes the file is not one. The history follows a move: a commit that adds the file with the blob another
path of the same commit removed (an exact rename, as a move keeps the blob id) is a version, and the versions before it
are those of the old path. Version names are full commit ids; a unique prefix of 7 or more digits also works. The
versions come from the branch's path index (below), not a walk from the head on each call.

### The path index

Each branch has a path index (`crates/moqui-core/src/resource/git/index.rs`): every commit of its first-parent line with
the files it changed against its first parent, and for each path (a file, or a directory with anything under it) the
last commit that changed it. It is built on first use, extended in place when the head moves forward (a commit, a push
to a `local` repository, a fetch), and built again when the branch was replaced (the old head is not on the new line).
At most 32 branches and about 256 MB are kept, the least used dropped first; a branch longer than 200,000 commits is
indexed back that far. It serves:

- `stat` and the last change of a path: a file's date and user are its own last commit's, not the head's.
- The version history, a version's text (read at the path the file had then), `#version` reads and prefixes.
- The log of the commits under a path, with the paths each changed there (`getChangeLog`).

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

Create a space with its own repository, `create#WikiSpace(wikiSpaceId: "DOCS", repositoryKind: "db")` (or `local` on
disk; `remote` with a `remoteUrl` is a read-only mirror, for administrators), which makes the `GitRepository`
`WIKI_DOCS` and roots the space at `git://WIKI_DOCS/DOCS.md`; or put it under an existing repository,
`create#WikiSpace(wikiSpaceId: "DOCS", rootPageDirectory: "git://docs")`, or point an existing
`WikiSpace.rootPageLocation` at `git://<repositoryId>/<SPACE>.md`. Each page save is then a commit (its `versionNote`
the message), and the page history, compare, publish and `#version` views work on commits. The gallery's Editor screen
makes such a space (`EXGIT`, in the `db` repository `example-wiki`) with its "Create the git space" action. The wiki
services are below, "Wiki pages".

### Limits

- **No push.** gitoxide, the git library, does not push yet. To publish a `db` repository, export it and push the
  export with the git command line; a `local` repository can be pushed from directly.
- **Transports.** HTTPS is built in (reqwest on rustls). A `file://` or local-path URL in a fetch runs the system
  `git-upload-pack`, and an `ssh://` URL the system `ssh`; import from a local repository directory or another
  `git://` repository needs neither, as it copies objects directly.
- SHA-1 repositories only. No submodules, LFS, or signed commits.
- The `git` cargo feature of `moqui-core` (on by default, off in the browser build) links gitoxide. Without it the
  `git` scheme is not installed.

## Version notes

A note is written with a version and read back as the version's `note`: on `git://` it is the commit message (without
its `Moqui-User-Id` trailer; a write with no note is `Create <path>` or `Update <path>`), on `dbresource://` it is
`DbResourceFile.versionNote`, kept on the version's `DbResourceFileHistory` row so it stays with the version when a
later write replaces it. Other storage writes the bytes and drops the note. A version is never relabeled: a commit
cannot be.

| From a script | |
|---|---|
| `ref.putText(text, note)`, `ref.putBytes(bytes, note)`, `ref.putStream(stream, note)` | A write with a note; without one they are as before |
| `ec.resource.putContent(location, content, note)` | The same for text, a `Uint8Array`, an uploaded file or a stream |
| `ec.resource.restoreVersion(location, versionName, note)` | That version written back as the newest (on `git://` one commit that puts the entry of that commit back, so a directory deleted later comes back whole); the new version's name |
| `ec.resource.getVersionBytes(location, versionName)` | A version's bytes as a `Uint8Array`, or null |

In Rust: `ResourceFacade::write_bytes_noted`, `restore_version`, and `ResourceProvider::write_bytes_noted` (a provider
that keeps no notes need not implement it).

## Change sets

A change set is several writes, deletes, moves and new directories under one root with one message:

```js
const result = ec.resource.makeChangeSet('git://docs/')
        .write('specs/a.md', 'text')              // text, a Uint8Array, an uploaded file or a stream
        .write('img/logo.png', uploadedFile)
        .delete('old')                            // a directory takes everything under it
        .move('draft.md', 'specs/final.md')       // the target must not exist
        .mkdir('empty')
        .commit('Reorganize the specs')           // {versionName, applied: [paths], failures: [{path, error}]}
```

`.author(userId, date)` before `commit` writes the versions as that user at that date (a timestamp or milliseconds), for
a version copied from other storage with its history: on `git://` the commit's author (name, email, date) and its
`Moqui-User-Id`, with the current user, now, as committer; on `dbresource://` the history row's `userId` and
`versionDate`. A git version's date is its author date, as `git log` shows it. In Rust,
`crate::context::with_version_author` around the writes.

On a `git://` root it is one commit, each change applied to the tree the changes before it made, with the message (or a
summary of the changes). Elsewhere each change applies in order: on `dbresource://` in the caller's transaction, with the
message as each written file's version note. A path is relative to the root; one that is empty or has a `.` or `..`
segment is refused. A change that fails (a move of nothing, a write onto a directory) is in `failures` and the others
still apply. A lone delete of a directory that still has files is still refused; in a change set it takes them.
`ResourceFacade::apply_change_set` and `ResourceProvider::apply_changes` are the Rust side.

## Last changes

| From a script | |
|---|---|
| `ec.resource.getLastChange(location)` | `{isFile, size, versionName, date, userId, note}` of a file or directory, null when nothing is there: on `git://` from the path index (a directory: its last commit under it), on `dbresource://` a file's current version |
| `ec.resource.getChildChanges(location)` | The same for each entry of a directory, with `name`, in name order (on `dbresource://` four queries for the whole folder) |
| `ec.resource.getChangeLog(location, limit)` | The versions under a location, newest first: on `git://` each commit under the path with the `paths` it changed; on `dbresource://` a file's versions, or a directory's files' versions merged by date |
| `ec.resource.getTreeSize(location)` | The bytes of every file under a location, for a quota |
| `ec.resource.contentTypeOf(filename)` | The content type of a file name |

`ResourceRef::size()` of a `dbresource://` file is its `DbResourceFile.fileSize`, written with each write; a row written
before the field existed is sized from its data until its next write.

## Drives

Files run on drives (`moqui.resource.ResourceDrive` and the entities beside it in `ResourceEntities.xml`), served by
`org.moqui.impl.FileServices` (`runtime/base-component/framework/service/org/moqui/impl/FileServices.xml`). A drive is a
root location with an owner, members and a quota:

| Kind | Root | Versions |
|---|---|---|
| Personal (`RdkPersonal`, made on first use by `ensure#PersonalDrive`) and folder drives (`RdkShared`, storage `folder`) | `dbresource://drive/personal/<userId>`, `dbresource://drive/shared/<driveId>`; the `moqui.drive.root` preference moves `dbresource://drive` | Each file's own, as history rows |
| Version-controlled (storage `gitDb` or `gitLocal`) | `git://DRIVE_<driveId>/`, a `GitRepository` of kind `db` or `local` made with the drive | Commits |
| Mirror (`RdkMirror`, storage `mirror`, admins only) | `git://DRIVE_<driveId>/` over a `remote` repository, fetched by `fetch_AllGitRepositories_frequent` | Commits; read only |
| Records (`RdkRecords`, set up by an application) | Its record attachment tree, for example `dbresource://mantle/content` | As stored |

**Access.** These are `moqui.*` entities, which entity filters never apply to, so the services check every call: a user
reaches a drive they own or are a current `ResourceDriveMember` of (a user or a user group) with `RdpView`, `RdpEdit` or
`RdpManage`; a `ResourceShare` to the user, one of their groups or their party grants its permission on its path and
below (`RspView`, `RspEdit`, or `RspUpload`: files may only be added). A mirror and an archived drive are read only. What
the user cannot reach is "not found". `get#DriveAccess` answers the level.

**Services.** Drives: `create#Drive`, `update#Drive`, `archive#Drive`, `add#DriveMember`, `remove#DriveMember`,
`get#DriveMembers`, `get#UserDrives` (with the bytes used), `export#Drive` (a `db` repository as a bare repository on
disk, a folder drive as a zip). Files: `list#Folder` (size, modified and modified by from the last change, version and
note, shared, favorite, tags, content and file type), `get#ResourceInfo`, `find#Resources` (through the `MoquiDriveFile`
search document when search is available, else a walk), `find#Recent`, `upload#Resource` (one change set, a version note,
conflicts replace, keepBoth or skip, the quota), `save#Text`, `create#Folder`, `rename#Resource`, `move#Resource` and
`copy#Resource` (within a drive one change set; a move across drives is a copy and a trash), `download#Zip`,
`get#Activity` (versions or commits under the path, shares, trash), `get#Preview` (an image thumbnail made once under
`runtime://tmp/preview`, a text excerpt; a PDF thumbnail through Chromium is later). Trash: `trash#Resource` (on the
database tree the item moves with its history into the drive's hidden `.trash/<trashId>/`; on git it is deleted in one
commit and `ResourceTrash.versionName` is the version before), `list#Trash`, `restore#Resource`,
`delete#TrashForever`, `empty#Trash`. Versions: `get#Versions`, `get#VersionDiff`, `restore#Version`. Sharing:
`share#Resource` (people or a link; a link's random token is returned once and only its hash kept, as the password),
`revoke#Share`, `accept#Share`, `decline#Share`, `get#Shares` (outgoing, links, incoming), `set#Favorite`,
`get#Favorites`, `set#Tags`, `get#Tags`. A move or rename carries the path's shares, favorites, tags and recent rows.

**Public links.** `check#PublicShareToken` (the token route's check), `open#PublicShare` (the token, then the password:
a grant for an hour, an HMAC of the share and its expiry keyed by the share's secrets; counts the opening),
`list#PublicShare`, `download#PublicShare` (a file, or a folder as a zip) and `upload#PublicShare` (a file drop with the
uploader's name in the version note; names never replace). Each is anonymous and checks the token before it reads
anything else, and never answers about a path outside the share.

**Records drive.** Its `ResourceDriveRecordFolder` rows map a top folder (or a pattern such as `asset/*/maintenance`) to
the record entity and id field of the folders below it; the application that writes those folders seeds them.
`list#Folder` shows the mapped folders at the top with their labels, and gives each record folder `recordEntityName`,
`idField` and `recordId`.

**Search.** `MoquiDriveFile` (index `moqui_drive`) has no row behind it: `index#DriveFiles` builds the documents (drive,
path, name, folder, content and file type, size, modified date and user, tags) on each upload, write, rename, move, copy,
tag change, restore and trash (which removes them), and `index#MirrorDrives` brings a mirror's documents to its head
after a fetch (the `Files.secas.xml` rules). `find#Resources` filters by the drives the user may open. Extracted text is
later.

## Wiki pages

`org.moqui.impl.WikiServices` (`runtime/base-component/framework/service/org/moqui/impl/WikiServices.xml`) keeps
knowledge spaces and each user's notes (`ensure#UserNotesSpace`, a private space on database files) over this API.
`WikiSpace` and `WikiPage` are `moqui.*` entities, which entity filters never apply to, so the services check the wiki's
own rules: `get#WikiPageAccess` (view, update, admin, and the storage of a space or page; a space with `restrictView`
through a `WikiSpaceUser` row, a page with it through a `WikiPageUser` row; the ADMIN group sees all, and internal work
with no user and authorization disabled does everything) and `get#WikiViewScope` (the spaces a user may view and the
restricted pages hidden from them, for the services that read many pages).

| Service | |
|---|---|
| `update#WikiPage(wikiSpaceId, pagePath or wikiPageId, pageName, parentPath or parentWikiPageId, pageText, versionNote)` | A new version with its note and a `WikiPageHistory` row; a new name or parent is a `move#WikiPage`. A path that is only the alias of a moved page is a new page there |
| `create#WikiSpace(..., repositoryKind, memberList)`, `clone#WikiSpace(baseWikiSpaceId, ..., repositoryKind, copyHistory)` | Above, "Wiki spaces on git"; the creator administers the space. A clone with history replays every version (below); without, each page once |
| `move#WikiPage(wikiPageId, toWikiSpaceId, toParentPath or toParentWikiPageId, toPageName, versionNote)` | The page with its child pages and attachments. On the same storage the files move (the database tree keeps the history rows; on git one commit, which the history follows); into other storage `copy#WikiPageFiles` replays each file's versions oldest first, one change set each by the version's own author at its own date (`.author`, above) with a note naming where it came from, then the old files go. The rows keep their ids (history, categories, users, comments and reviews stay); a replay renames their version names; a `WikiPageHistory` row records the move with `oldWikiSpaceId` and `oldPagePath`, and a `WikiPageAlias` at the old path keeps links to it working |
| `get#WikiPageHistory`, `get#WikiPageVersionDiff(fromVersionName, toVersionName)`, `restore#WikiPageVersion(versionName, versionNote)` | The History tab: versions with their notes and who wrote them, the server's line diff, and a restore as a new version |
| `get#WikiPageBacklinks`, `get#WikiPageLinks` | `WikiPageLink` rows, indexed from each save's text (markdown links, reference definitions, `[[path]]`, `wiki:SPACE/path`; relative paths from the page's parent; not in code); a page made at a path takes the links written to it, a moved page keeps its backlinks through its alias, a deleted one leaves links to a page not there. A text that cannot be read for links is logged and the save stands |
| `request#WikiPageReview(versionName, reviewerUserIdList, dueDate, note, link)`, `decide#WikiPageReview(decision approve or changes, decisionNote, publish)`, `cancel#WikiPageReview`, `find#WikiPageReviews(filter toDo, requested or all, wikiSpaceId, statusId)` | `WikiPageReview` rows (`WprRequested`, then `WprApproved`, which publishes the version, `WprChangesRequested` or `WprCancelled`); the reviewer is told on topic `WikiPageReview`, an approval the tray decides (`approve#WikiPageReview`, `refuse#WikiPageReview` with the reason), the requester on `WikiPageReviewDecision` |
| `search#WikiPages(text, wikiSpaceId)` | Through any search document on `WikiPage` (its content, title and last change from `get#WikiPageManualDocumentData`) when search is available, else the page paths; never a page the user may not view |
| `get#WikiSpacePages(includeDetail)` | The tree, without the restricted pages a user may not view; with the detail each page's title, snippet, last change, categories and pinned (the category `Pinned`), for a notes list |

## Token routes and the public shell

A `token-route` of the webroot `webapp` in the conf makes `/<name>/<token>` an anonymous page:

```xml
<webapp-list><webapp name="webroot">
    <token-route name="s" screen-location="component://my-app/screen/PublicShare.xml"
            token-service="org.moqui.impl.FileServices.check#PublicShareToken" title="Shared files"/>
</webapp></webapp-list>
```

The token service runs first, with `token` and `routeName`, and answers `valid` and `routeContext`; a token it does not
accept gets a short "link expired or not found" page (404) and nothing of the screen runs. Otherwise `/<name>/<token>` is
the public shell (the webroot screen `PublicShell`, `component://webroot/screen/webroot/PublicShell.xml`, under the webroot header and footer): Vue, Quasar, the theme's style sheets and the `m-*`
components of `WebrootVue.qvt.js`, with no header, drawers or session user; `js/WebrootPublic.qvt.js` mounts the route's
screen, loaded as `/<name>/<token>.qvt` (or `.qvue`, `.qjs`, by the screen's render modes), and its transitions are
under `/<name>/<token>/`. The screen's context has `token` and `tokenRoute` (`{name, token, basePath, title,
context}`); a request parameter of either name is replaced. A route named as a framework path (`apps`, `qapps`, `rest`,
...) is left out, and a component's route of the same name replaces an earlier one. The services a public screen calls
check the token again. `runtime/base-component/example/screen/PublicLink.xml` is a minimal route screen.
