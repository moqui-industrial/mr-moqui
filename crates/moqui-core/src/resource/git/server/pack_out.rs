/*
 * This software is in the public domain under CC0 1.0 Universal plus a
 * Grant of Patent License.
 *
 * To the extent possible under law, the author(s) have dedicated all
 * copyright and related and neighboring rights to this software to the
 * public domain worldwide. This software is distributed without any
 * warranty.
 *
 * You should have received a copy of the CC0 Public Domain Dedication
 * along with this software (see the LICENSE.md file). If not, see
 * <http://creativecommons.org/publicdomain/zero/1.0/>.
 */

//! The pack data a fetch sends: version 2, SHA-1, the planned objects. A repository on disk gives gix-pack's
//! generator its object database, so objects already stored as deltas in a pack go out as those deltas (with their
//! base in the same pack); a client that did not ask for `ofs-delta` gets whole objects. The `db` kind (and any other
//! store) writes whole objects, read and compressed on this thread: gix's generator reads on worker threads, which have
//! no Moqui context to read rows in.

use std::io::Write;

#[cfg(test)]
use gix::objs::Kind;
use gix::odb::pack::data::output::{self, Count};
use gix::ObjectId;

use crate::context::Error;
use crate::resource::git::store::GitStore;

fn pack_error(what: &str, err: impl std::fmt::Display) -> Error {
    Error::new(format!("{what}: {err}"))
}

/// Write `objects` to `out` as a pack. Every object must be in the store: a pack's object count is written first.
pub(crate) fn write_pack(
    store: &dyn GitStore,
    objects: &[ObjectId],
    ofs_delta: bool,
    threads: Option<usize>,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let count = u32::try_from(objects.len()).map_err(|_| Error::new("too many objects for one pack"))?;
    if let (true, Some(handle)) = (ofs_delta, store.pack_handle()) {
        return write_from_packs(handle, objects, count, threads, out);
    }
    write_whole(store, objects, count, out)
}

/// Through gix-pack's generator over a repository's object database, copying packed entries (deltas included).
fn write_from_packs(
    handle: gix::odb::store::Handle<std::sync::Arc<gix::odb::Store>>,
    objects: &[ObjectId],
    count: u32,
    threads: Option<usize>,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let counts: Vec<Count> = objects
        .iter()
        .map(|id| Count {
            id: *id,
            entry_pack_location: output::count::PackLocation::NotLookedUp,
        })
        .collect();
    let entries = output::entry::iter_from_counts(
        counts,
        handle,
        Box::new(gix::progress::Discard),
        output::entry::iter_from_counts::Options {
            thread_limit: threads,
            mode: output::entry::iter_from_counts::Mode::PackCopyAndBaseObjects,
            allow_thin_pack: false,
            chunk_size: 64,
            version: gix::odb::pack::data::Version::V2,
            compression: gix::zlib::Compression::DEFAULT,
        },
    )
    .map_err(|err| pack_error("preparing the pack", err))?;
    let mut writer = output::bytes::FromEntriesIter::new(
        gix::parallel::InOrderIter::from(entries),
        out,
        count,
        gix::odb::pack::data::Version::V2,
        gix::hash::Kind::Sha1,
    );
    for written in writer.by_ref() {
        written.map_err(|err| pack_error("writing the pack", err))?;
    }
    let stats = gix::parallel::Finalize::finalize(writer.input.inner)
        .map_err(|err| pack_error("writing the pack", err))?;
    if stats.missing_objects > 0 {
        return Err(Error::new(format!(
            "{} objects were missing while the pack was written",
            stats.missing_objects
        )));
    }
    Ok(())
}

/// Whole objects, each read and compressed here.
fn write_whole(store: &dyn GitStore, objects: &[ObjectId], count: u32, out: &mut dyn Write) -> Result<(), Error> {
    let io = |err: std::io::Error| pack_error("writing the pack", err);
    let mut hashed = gix::hash::io::Write::new(out, gix::hash::Kind::Sha1);
    hashed
        .write_all(&gix::odb::pack::data::header::encode(gix::odb::pack::data::Version::V2, count))
        .map_err(io)?;
    for id in objects {
        let (kind, data) = store
            .find(id)?
            .ok_or_else(|| Error::new(format!("git object {id} is missing")))?;
        let entry = output::Entry::from_data(
            &Count::from_data(*id, None),
            &gix::objs::Data::new(&data, kind, gix::hash::Kind::Sha1),
            gix::zlib::Compression::DEFAULT,
        )
        .map_err(|err| pack_error(&format!("compressing {id}"), err))?;
        let header = entry.to_entry_header(gix::odb::pack::data::Version::V2, |_| 0);
        header
            .write_to(entry.decompressed_size as u64, &mut hashed)
            .map_err(io)?;
        hashed.write_all(&entry.compressed_data).map_err(io)?;
    }
    let digest = hashed
        .hash
        .try_finalize()
        .map_err(|err| pack_error("hashing the pack", err))?;
    hashed.inner.write_all(digest.as_slice()).map_err(io)?;
    hashed.inner.flush().map_err(io)
}

/// Whether every object of `objects` that is not a commit or tree is in the store (commits and trees were read while
/// planning). The ids that are missing.
pub(crate) fn missing_objects(store: &dyn GitStore, objects: &[ObjectId]) -> Result<Vec<ObjectId>, Error> {
    let mut missing = Vec::new();
    for chunk in objects.chunks(500) {
        let present = store.contains_many(chunk)?;
        missing.extend(chunk.iter().filter(|id| !present.contains(*id)));
    }
    Ok(missing)
}

/// Read a pack written here back, for tests: every object's id and kind, in pack order.
#[cfg(test)]
pub(crate) fn read_back(pack: &[u8]) -> Vec<(ObjectId, Kind)> {
    let dir = std::env::temp_dir().join(format!(
        "moqui-pack-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos())
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let interrupt = std::sync::atomic::AtomicBool::new(false);
    let outcome = gix::odb::pack::Bundle::write_to_directory(
        &mut std::io::BufReader::new(pack),
        Some(&dir),
        &mut gix::progress::Discard,
        &interrupt,
        None::<gix::objs::find::Never>,
        gix::hash::Kind::Sha1,
        gix::odb::pack::bundle::write::Options::default(),
    )
    .unwrap();
    let bundle = outcome.to_bundle().unwrap().unwrap();
    let mut buf = Vec::new();
    let mut found = Vec::new();
    for entry in bundle.index.iter() {
        let (data, _) = bundle
            .find(&entry.oid, &mut buf, &mut gix::zlib::Inflate::default(), &mut gix::odb::pack::cache::Never)
            .unwrap()
            .unwrap();
        found.push((entry.oid, data.kind));
    }
    let _ = std::fs::remove_dir_all(&dir);
    found
}
