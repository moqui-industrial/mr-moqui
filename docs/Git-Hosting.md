# Git hosting

MR Moqui hosts the `moqui.resource.GitRepository` repositories of [Revision-Management](Revision-Management.md) for the
git command line and any git client: clone, fetch and push over HTTPS (smart HTTP) and over the built-in SSH server,
shallow clones for CI, branches and tags, and Git LFS. A push or a mirror fetch runs **gitECA rules**, the git
counterpart of email ECA rules, and Mantle uses them to tie commits, branches and tags to projects and tasks. gitoxide
has no server side; the protocol is ours (`crates/moqui-core/src/resource/git/server/`) over gitoxide's objects and
packs.

## Clone URLs

| | URL |
|---|---|
| HTTPS | `https://<host>/git/<repositoryId>.git` (`.git` is optional) |
| SSH | `ssh://<username>@<host>:<ssh_port>/<repositoryId>.git`, or `<username>@<host>:<repositoryId>.git` on port 22 |

`org.moqui.impl.GitServices.get#GitCloneUrls` gives both, from `git_public_url` (else the webapp host and port
properties), and `ssh_public_host` and `ssh_public_port` (else the webapp host and `ssh_port`). The SSH username is
the Moqui username. The kinds of repository behave as they do for `git://` locations:

- **`local`**: a bare repository on disk, the one for code. Clones reuse the deltas of its packs.
- **`db`**: objects in `GitObject` rows, kept with the database backup, compressed and as deltas of each other as a
  pack holds them; a clone copies them into its pack as they are ([Storage](#storage-of-db-repositories)). Large files
  are best kept in [Git LFS](#large-files-git-lfs).
- **`remote`**: a read-only mirror; it can be cloned and fetched, a push is refused. A fetch of the mirror takes every
  branch and tag and drops the ones the remote dropped.

A repository is created on the Git Repositories tools screen (System, Resource Finder), by
`mantle.work.GitServices.create#ProjectRepository` from a project's Repositories tab, or as a `GitRepository` row.
An empty repository can be cloned; the first push to one whose row names no branch makes `main` (or `master`, or the
first branch pushed) its default branch.

## Logging in

- **HTTPS**: HTTP Basic with the username and the password, or an **access token** as the password, which accounts
  with a second factor need (the password alone cannot complete a login there). `Authorization: Bearer <token>` and
  an `api_key` header work too. Credentials are taken only over HTTPS (the `webapp_https_listen_port` listener, or
  `X-Forwarded-Proto: https` behind the proxy that `webapp_client_ip_header` names), from loopback, or with
  `git_allow_http=true`. A request with no credentials gets 401, so git asks for them.
- **SSH**: a key (`moqui.security.UserSshKey`, matched by its SHA-256 fingerprint) or the password. The user needs the
  `SSH_GIT` permission (granted to ADMIN; grant it to the groups of developers). A user whose only SSH permission is
  `SSH_GIT` logs in by key unless `ssh_git_password_auth=true`. `git -c protocol.version=2` sends `GIT_PROTOCOL`,
  which the server reads from the channel's `env`.

My Account, Git Access (`MyAccount/User/GitAccess.xml` of SimpleScreens) adds and removes SSH keys and creates and
revokes access tokens, through `org.moqui.impl.UserServices` `create#UserSshKey`, `delete#UserSshKey`,
`create#UserAccessToken` (a `UserLoginKey` with a description; the token is shown once) and `revoke#UserAccessToken`.

## Access

Each repository is an artifact of type `AT_GIT_REPOSITORY`, named by its `repositoryId`, in the usual ArtifactGroup
and ArtifactAuthz rows:

| Action | Allows |
|---|---|
| `AUTHZA_VIEW` | clone, fetch, and the `get#Git*` services |
| `AUTHZA_CREATE` | push a new branch or tag |
| `AUTHZA_UPDATE` | push a fast-forward, and `commit#GitFiles` |
| `AUTHZA_DELETE` | force a push (a branch that does not fast-forward, a moved tag) and delete a branch or tag |

The seed data has the `GitRepositories` group (every repository, `.*`) with ALWAYS ALL for ADMIN. Mantle adds an
ALLOW ALL for ALL_USERS on it whose `authzServiceName` is `mantle.work.GitServices.check#GitRepositoryAuthz`: the
active parties of a project a repository is linked to may view, create and update (not delete). A user who may not
view a repository gets 404 as if it were not there; one who may view and not push gets 403 on a push. The server
refuses to serve at all when `<artifact-execution type="AT_GIT_REPOSITORY" authz-enabled="true"/>` is not in the conf,
as every check would pass then. Reading and writing `git://` locations is not checked against this type.

The default branch cannot be deleted. Only `refs/heads/` and `refs/tags/` can be pushed.

## Protocol

- **Fetch**: protocol v2 (`ls-refs`, `fetch`) and v0/v1 (`multi_ack_detailed`, `no-done`, `side-band-64k`), stateful
  over SSH and stateless over HTTP; `shallow`, `deepen`, `deepen-relative` (`--deepen`), `deepen-since`
  (`--shallow-since`), `deepen-not` (`--shallow-exclude`) and `--unshallow`; `include-tag`; a want of any object the
  repository has (`git fetch --depth=1 origin <sha>`). No partial clone (`filter`) yet.
- **Push**: v0/v1 (v2 has no push), `report-status`, `side-band-64k`, `atomic`, `delete-refs`, `push-options`. The
  pack is indexed into a quarantine directory, completed from the repository when it is thin, and every object it
  refers to must be in it or the repository; only an accepted push moves it in (a `local` repository keeps the pack in
  `objects/pack`, a `db` one writes each object as a row). Each ref is then moved by compare-and-set: a ref another
  push moved first is refused with `stale info`.
- Clones and pushes run on a pool of their own (`git_max_concurrent`, 4, and `git_max_queued` waiting), not on the
  request workers of the screens, and are exempt from `webapp_request_timeout`; `git_idle_timeout` (120 seconds) of
  silence in a request body ends it. Limits: `git_max_request` (64 MB) for a fetch request, `git_max_push_size` (2 GB),
  `git_max_object_size` (512 MB) for one pushed object; `git_pack_threads` (all cores when empty) compress and index.

## gitECA rules

A `*.gitecas.xml` file in a component's `service` directory (schema `framework/xsd/git-eca-3.xsd`):

```xml
<gitecas>
    <giteca rule-name="NoWip" event="pre-receive">
        <condition><expression>newCommits.some(c => c.summary.startsWith('WIP'))</expression></condition>
        <actions><message error="true">WIP commits are not accepted</message></actions>
    </giteca>
    <giteca rule-name="ProtectMain" event="pre-ref-update" ref-pattern="refs/heads/main">
        <condition><expression>changeType == 'force'</expression></condition>
        <actions><message error="true">main cannot be forced</message></actions>
    </giteca>
    <giteca rule-name="LinkCommitTasks" event="commit">
        <actions><service-call name="mantle.work.GitServices.link#CommitReferences" in-map="context"/></actions>
    </giteca>
</gitecas>
```

| Event | Runs | An error |
|---|---|---|
| `pre-receive` | once per push, before any ref changes, with `newCommits` | refuses the push |
| `pre-ref-update` | once per ref of a push, before it changes | refuses that ref (every ref of an `atomic` push) |
| `ref-update` | once per ref that changed, with `newCommitIds` (the commits it gained) | is recorded |
| `commit` | once per commit new to the repository, oldest first, with the ref it came on | is recorded |
| `tag` | once per tag created | is recorded |
| `post-receive` | once per change, after the others | is recorded |

| Attribute | |
|---|---|
| `source` | `push`, `fetch` (of a `remote` mirror), `import`, `write` (a commit through `git://` or `commit#GitFiles`, such as a wiki save); `push,fetch,import` when empty |
| `repository-pattern`, `ref-pattern` | regular expressions the whole repositoryId, or ref name (`refs/heads/main`), must match |
| `ref-type` | `branch` or `tag` |
| `default-branch-only` | the repository's default branch only (a commit event: a commit that came on it) |
| `priority`, `id`, `run-on-error` | as for service ECA rules: lower first; a later rule with the same id replaces this one; run even after a rule of the same change failed |

The context of every event has `repositoryId`, `source`, `transport` (`http`, `ssh`, `internal`), `userId`,
`updateId`, `pushOptions` and `refUpdates` (each `refName`, `refType`, `shortName`, `oldId`, `newId`, `changeType` of
`create`, `update` (a fast-forward), `force` or `delete`, `isDefaultBranch`); a per-ref event those fields of its ref;
a commit event `commit` (`commitId`, `shortId`, `summary`, `body`, `message`, `trailers` by name, the author and
committer and their dates, `parentIds`, `moquiUserId`); a tag event `tag` (`tagName`, `targetId`, `commitId`,
`annotated`, and an annotated tag's `message` and tagger). `org.moqui.GitServices.process#GitEca` is the interface.

The pre events run in the push, against its quarantined objects: their messages, and the reasons of a refusal, show
on the client as `remote:` lines. The post events run after the change, on a job of the git pool, from its
`moqui.resource.GitUpdate` row (with a `GitUpdateRef` per ref), each rule in a transaction of its own; one that fails
is logged and recorded on the row (`ecaStatus` failed, `ecaError`), and the others still run. The updates of one
repository run one at a time, in order. Every push is recorded, accepted or not, so the rows are also its push log
(the tools screen of the repository lists them, and runs a done or failed one again). A change's commit events stop at
`git_eca_max_commits` (10,000) new commits. The `process_PendingGitUpdates_frequent` job runs what is left pending (a
full pool, a restart) and finishes a push a restart cut short. Rules run with authorization off: they are a
component's own code.

## Projects and tasks

In Mantle (`mantle.work.GitServices`, the rules of `mantle-usl/service/Work.gitecas.xml`, and the screens of
SimpleScreens), a project's Repositories tab links repositories to it (`WorkEffortGitRepository`), or makes a new one.
In a repository linked to a project:

| | Links |
|---|---|
| A task ID of the project as a word of a commit message (`PROJ-100023`) | the commit to the task (`WorkEffortGitCommit`). A word is looked up, so `UTF-8` never matches |
| A `Task: <workEffortId>` trailer | the commit to that task, in any project |
| `Fixes`, `Closes` or `Resolves` before a task ID | completes and resolves the task (`WeComplete`, `WerCompleted`) when the commit reaches the default branch: only commits made after the repository was linked, and only where the task's status flow has a way to Complete |
| A branch named `PROJ-100023` or `PROJ-100023-login-fix` (its last path segment, or the part of it before a `-`) | the branch to the task (`WorkEffortGitRef`) |
| A tag named `<workEffortId>` or `<workEffortId>/<anything>`: `PROJ/v1.2.0`, `PROJ-MS-01/rc1`, `PROJ-100023/review` | the tag to that project, milestone or task; an annotated tag's message is read for task IDs as a commit's is |

Deleting a branch or tag ends its links (`thruDate`). The rules run on pushes, mirror fetches (so a repository hosted
elsewhere and mirrored here links too), imports and commits made through Moqui. A task's Source tab links commits,
branches and tags by hand; its summary shows them, and the project summary the repositories and tags.
`Project/GitCommit` shows a commit: its message, people, parents, linked tasks, and each changed file's diff.

## Storage of db repositories

A `GitObject` row holds an object as a pack does (`storage`):

| `storage` | `objectData` |
|---|---|
| `zlib` | the zlib stream of the content |
| `delta` | the zlib stream of git delta instructions that make the content from `deltaBaseId`, an object of the same repository, at most `git_db_delta_depth` (50) deltas from a whole one |
| `blob` | nothing: an object over `git_db_large_object` bytes (8 MiB) is the [content blob](#content-blobs) `blobId` |
| empty | the content itself, as rows written before this format were; still read, and compressed by the next collection |

A push keeps the deltas the client sent, and its entries' compressed bytes, as they are (a commit or tag, and a delta
deeper than the limit, is stored whole); a large object is inflated into its content blob as the pack is read. A clone
or fetch copies the rows into its pack: a delta as `OFS_DELTA` when its base is in the pack (or `REF_DELTA` without
`ofs-delta`), as a delta against an object the client has in a thin pack, and applied otherwise; a content blob is
compressed into the pack a window at a time. Reads go a batch of rows at a time, a delta chain a level of bases per
query, and each object is checked against its id as it is decoded; `git_db_cache_bytes` (64 MiB) of decoded objects
are kept for all repositories. `import#GitRepository` and `export#GitRepository` go through a pack as a push does.
`git_db_write_format=raw` writes rows in the old form, for a database an older version also reads.

`gc#GitRepositories` (the `gc_GitRepositories_daily` job, 03:30) collects each `db` repository, holding its lease
(`gcLeaseOwner`) so only one run rewrites its rows:

1. It marks what the refs reach, and what may still be needed: the tips of a change whose gitECA events have not run,
   the old and new ids of the changes of the last `git_gc_reflog_days` (30) days (a force push can be undone), and every
   row written or referenced again in the last `git_gc_prune_days` (14) days. A ref that reaches a missing object stops
   the run: a damaged repository is never swept (`fsck#GitRepository` names what is wrong).
2. It deletes the other rows, deepest delta first, each only while it is still that old: a push that reuses an old
   object while the run goes refreshes its stamp, and keeps it.
3. It repacks while its time (`timeoutMinutes`, 60) lasts: the versions of each path, newest first, the newest whole
   and each older one a delta of a newer one when that saves a quarter of its bytes; rows of the old form compressed;
   chains deeper than the limit made shorter. Each new delta is applied and checked against the object's id first.
4. It lets go of the LFS objects no kept pointer names, uploaded before the cutoff, and records `objectCount`,
   `storedBytes`, `lastGcDate` and `lastGcResult` on the repository. `dryRun` counts and changes nothing.

The tools screen of a `db` repository shows its storage and runs a collection (a dry run by default) and the check.

Measured on embedded Postgres (debug build, `cargo test -p moqui-host --test it -- --ignored git_storage
--nocapture`):

| | Objects | Content | git's pack | Stored, imported | After a collection | Clone |
|---|---|---|---|---|---|---|
| mr-mantle-usl's history | 13,435 | 195 MiB | 8.25 MiB | 8.37 MiB (1.02x) | 7.88 MiB (0.96x) | 3 to 5 s |
| mr-moqui's history | 11,404 | 160 MiB | 10.3 MiB | 25.6 MiB | 10.4 MiB (1.01x) | 2 to 3 s |
| 1000 `git://` saves of a 50 KB page | about 3,000 | about 49 MiB | | 2.72 MiB | 0.30 MiB | |

Before this format the rows held the content (the Content column). A 1 GiB file is stored as it is read (55 MiB/s)
and sent at about 270 MiB/s, the process's peak memory rising about 180 MiB; a 64 KiB range of it takes about 50 ms.

## Content blobs

Large content is a `moqui.resource.ContentBlob`, by the SHA-256 of its bytes, in `ContentBlobChunk` rows of
`blob_chunk_size` (1 MiB) written a chunk per transaction: Git LFS objects, the large objects of `db` repositories, and
`dbresource://` files over 64 KiB. The same bytes are stored once; two uploads of them at once, or a crash halfway
(`ContentBlobUpload`), leave one blob and no stray chunks. A row refers to a blob by a relationship to `ContentBlob`,
and `gc#ContentBlobs` (`org.moqui.impl.ResourceServices`, the `gc_ContentBlobs_daily` job, 04:30) deletes the blobs no
row refers to and that were not written or referenced again in the last `blob_gc_prune_days` (14) days.

File responses stream (`sendResourceResponse`, a screen's static files): a file over 256 KiB is read a window at a time
as the client takes it, on a thread of its own (at most `webapp_transfer_workers`, 16, reading the database at once), so
a slow download holds no request worker. One `Range` is answered from the bytes it names, with `ETag` (a git file's
blob id, a content blob's SHA-256) and `If-Range`. Only text types are compressed. An upload is read as it arrives: a
file part over 256 KiB goes to a file in `runtime/tmp/upload`, and from there to its storage (a content blob, a git
object) as it is read; `webapp_max_upload` (2 GiB) bounds it and `webapp_upload_idle_timeout` (60 seconds) of silence
ends it.

## Large files (Git LFS)

git-lfs clients push and fetch large files at `/git/<repositoryId>.git/info/lfs` (the Batch API with the basic
transfer, and the locks API): `git lfs install`, `git lfs track "*.psd"`, then push and clone as usual. Over SSH,
git-lfs runs `git-lfs-authenticate`, which gives it that HTTPS endpoint and a token for an hour (`git-lfs-transfer` is
refused, so it does). An object is the content blob of its SHA-256, so the same file in many repositories is stored once, but a
repository has an object (`moqui.resource.GitLfsObject`) only once it was uploaded to it: knowing an oid does not reach
another repository's file. Uploads need the push rights of the repository, downloads view; a lock needs push rights,
and force unlocking another user's lock `delete`. The batch's actions carry a token signed with `git_lfs_token_key`
(generated into `runtime/.env`; `git_lfs_token_key_previous` still reads while it is changed) that says who the user
is, for 15 minutes, and each request is authorized again. Objects are at most `git_lfs_max_object_size` (5 GiB).

Moqui itself writes LFS files through `git://` locations: a file the `.gitattributes` of the branch head keep in LFS
(`filter=lfs`) is stored as a content blob, linked to the repository, and committed as its pointer; a read gives the
content, streamed for a response, and its size is the content's. `setup#GitLfs(repositoryId, patterns)` adds the
patterns to `.gitattributes` (images, video, audio, PDFs and archives when empty); a content repository gets them when
it is made (`init#GitRepository` with `contentRepository`, the tools screen's Content option, and the Content option of
a project's new repository). A web site's images and their variants (`convert#Image` writes an image and its sizes in
one commit) can so live in a `db` repository, with their history, in the database.

## Limits

- No push from a repository to another server; publish a `db` repository with `export#GitRepository`.
- A push made directly to a `local` repository's directory on the host runs no gitECA rules.
- Each push to a `local` repository adds a pack; there is no repack of those yet (run `git repack -ad` in the
  directory). `gc#GitRepositories` is for `db` repositories.
- `local` and `remote` repositories are on one node's disk: in a cluster send `/git` and SSH to that node, or share
  the directory. `db` repositories and content blobs are in the database, for every node.
- A `remote` mirror does not fetch the LFS objects of the repository it mirrors.
- SHA-1 only; no partial clone, signed pushes (`push-cert`), `report-status-v2`, the dumb HTTP protocol, or LFS
  transfer adapters other than basic.
