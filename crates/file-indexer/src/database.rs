//! SQLite-owned metadata; never modifies indexed source files.
mod context;
use crate::index_scan::{Cache, IndexedFile, Snapshot, path_bytes, path_from_bytes};
pub use context::{FileTypeTotal, FolderContext, IndexOverview};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
pub struct Index {
    connection: Connection,
}
#[derive(Debug, Serialize)]
pub struct Scope {
    pub id: i64,
    pub path: String,
    pub status: String,
    pub scanned_at: Option<i64>,
    pub files: i64,
    pub bytes: i64,
    pub omitted: i64,
    pub content: bool,
}
#[derive(Debug, Serialize)]
pub struct FileRow {
    pub id: i64,
    pub path: String,
    pub size: i64,
    pub modified: i64,
    pub excerpt: Option<String>,
    pub hashed: bool,
}
#[derive(Debug, Serialize)]
pub struct Page {
    pub files: Vec<FileRow>,
    pub total: i64,
}
#[derive(Debug, Clone, Serialize)]
pub struct ScopeFileRecord {
    pub id: i64,
    pub path: PathBuf,
    pub display: String,
    pub identity: String,
    pub size: u64,
    pub modified: i64,
    pub excerpt: Option<String>,
    pub hash: Option<String>,
}
impl Index {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(3))?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA cache_size=-8192; PRAGMA secure_delete=ON;")?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "Index schema is newer than this application".into(),
            ));
        }
        if version == 0 {
            connection.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE scopes(id INTEGER PRIMARY KEY AUTOINCREMENT, path BLOB NOT NULL UNIQUE, display TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'not scanned', scanned_at INTEGER, generation INTEGER NOT NULL DEFAULT 0, omitted INTEGER NOT NULL DEFAULT 0, content INTEGER NOT NULL DEFAULT 0, root_identity TEXT NOT NULL);
CREATE TABLE files(id INTEGER PRIMARY KEY AUTOINCREMENT, scope_id INTEGER NOT NULL REFERENCES scopes(id) ON DELETE CASCADE, path BLOB NOT NULL, display TEXT NOT NULL, identity TEXT NOT NULL, fingerprint TEXT NOT NULL, size INTEGER NOT NULL, modified INTEGER NOT NULL, excerpt TEXT, hash TEXT, generation INTEGER NOT NULL, UNIQUE(scope_id,path));
CREATE INDEX file_identity ON files(scope_id,identity);
CREATE INDEX file_generation ON files(scope_id,generation);
CREATE VIRTUAL TABLE search USING fts5(display,excerpt,content='files',content_rowid='id');
CREATE TRIGGER files_insert AFTER INSERT ON files BEGIN INSERT INTO search(rowid,display,excerpt) VALUES(new.id,new.display,new.excerpt); END;
CREATE TRIGGER files_delete AFTER DELETE ON files BEGIN INSERT INTO search(search,rowid,display,excerpt) VALUES('delete',old.id,old.display,old.excerpt); END;
CREATE TRIGGER files_update AFTER UPDATE ON files BEGIN INSERT INTO search(search,rowid,display,excerpt) VALUES('delete',old.id,old.display,old.excerpt); INSERT INTO search(rowid,display,excerpt) VALUES(new.id,new.display,new.excerpt); END;
PRAGMA user_version=1; COMMIT;")?;
        }
        Ok(Self { connection })
    }
    pub fn add_scope(&self, root: &crate::AuthorizedRoot) -> rusqlite::Result<i64> {
        let path = root.path();
        let identity = root_identity(root);
        self.connection.execute(
            "INSERT INTO scopes(path,display,root_identity) VALUES(?1,?2,?3) ON CONFLICT(path) DO UPDATE SET root_identity=excluded.root_identity",
            params![path_bytes(path), path.to_string_lossy(), identity],
        )?;
        self.connection.query_row(
            "SELECT id FROM scopes WHERE path=?1",
            [path_bytes(path)],
            |r| r.get(0),
        )
    }
    pub fn root(&self, id: i64) -> rusqlite::Result<PathBuf> {
        self.connection
            .query_row("SELECT path FROM scopes WHERE id=?1", [id], |r| {
                Ok(path_from_bytes(r.get(0)?))
            })
    }
    pub fn matches_root(&self, id: i64, root: &crate::AuthorizedRoot) -> rusqlite::Result<bool> {
        let stored: String = self.connection.query_row(
            "SELECT root_identity FROM scopes WHERE id=?1",
            [id],
            |r| r.get(0),
        )?;
        Ok(stored == root_identity(root))
    }
    pub fn scopes(&self) -> rusqlite::Result<Vec<Scope>> {
        self.connection.prepare("SELECT s.id,s.display,s.status,s.scanned_at,COUNT(f.id),COALESCE(SUM(f.size),0),s.omitted,s.content FROM scopes s LEFT JOIN files f ON f.scope_id=s.id AND f.generation=s.generation GROUP BY s.id ORDER BY s.id")?.query_map([], |r| Ok(Scope { id:r.get(0)?,path:r.get(1)?,status:r.get(2)?,scanned_at:r.get(3)?,files:r.get(4)?,bytes:r.get(5)?,omitted:r.get(6)?,content:r.get(7)? }))?.collect()
    }
    pub fn forget(&self, id: i64) -> rusqlite::Result<()> {
        self.connection
            .execute("DELETE FROM scopes WHERE id=?1", [id])?;
        // Logical purge plus FTS segment merge and WAL checkpoint. Not a forensic-erasure guarantee.
        self.connection.execute_batch(
            "INSERT INTO search(search) VALUES('optimize'); PRAGMA wal_checkpoint(TRUNCATE);",
        )?;
        Ok(())
    }
    pub fn cache(&self, id: i64) -> rusqlite::Result<Cache> {
        self.root(id)?;
        self.connection.prepare("SELECT f.path,f.identity,f.fingerprint,f.size,f.modified,f.excerpt,f.hash FROM files f JOIN scopes s ON s.id=f.scope_id WHERE s.id=?1 AND f.generation=s.generation LIMIT 100000")?.query_map([id], |r| {
            let path: Vec<u8> = r.get(0)?;
            Ok((path.clone(), IndexedFile { path:path_from_bytes(path),identity:r.get(1)?,fingerprint:r.get(2)?,size:r.get::<_,i64>(3)? as u64,modified:r.get(4)?,excerpt:r.get(5)?,hash:r.get(6)? }))
        })?.collect()
    }
    pub fn commit(&mut self, id: i64, snapshot: &Snapshot, content: bool) -> rusqlite::Result<()> {
        let tx = self.connection.transaction()?;
        let generation: i64 =
            tx.query_row("SELECT generation+1 FROM scopes WHERE id=?1", [id], |r| {
                r.get(0)
            })?;
        let paths: HashSet<_> = snapshot.files.iter().map(|f| path_bytes(&f.path)).collect();
        let mut identities = HashMap::new();
        for file in &snapshot.files {
            *identities.entry(&file.identity).or_insert(0) += 1;
        }
        for file in &snapshot.files {
            let path = path_bytes(&file.path);
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM files WHERE scope_id=?1 AND path=?2",
                    params![id, path],
                    |r| r.get(0),
                )
                .optional()?;
            if existing.is_none() && !file.identity.is_empty() && identities[&file.identity] == 1 {
                let old: Vec<(i64, Vec<u8>)> = tx
                    .prepare("SELECT id,path FROM files WHERE scope_id=?1 AND identity=?2 LIMIT 2")?
                    .query_map(params![id, file.identity], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                if old.len() == 1 && !paths.contains(&old[0].1) {
                    tx.execute(
                        "UPDATE files SET path=?1 WHERE id=?2",
                        params![path, old[0].0],
                    )?;
                }
            }
            tx.execute("INSERT INTO files(scope_id,path,display,identity,fingerprint,size,modified,excerpt,hash,generation) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(scope_id,path) DO UPDATE SET display=excluded.display,identity=excluded.identity,fingerprint=excluded.fingerprint,size=excluded.size,modified=excluded.modified,excerpt=excluded.excerpt,hash=excluded.hash,generation=excluded.generation",params![id,path,file.path.to_string_lossy(),file.identity,file.fingerprint,file.size.min(i64::MAX as u64) as i64,file.modified,file.excerpt,file.hash,generation])?;
        }
        if snapshot.complete {
            tx.execute(
                "DELETE FROM files WHERE scope_id=?1 AND generation<>?2",
                params![id, generation],
            )?;
        }
        // Hide old generations after partial scans; retain their metadata for reconciliation.
        // Remove stale content even on partial scans so it cannot survive a newly protected subtree.
        tx.execute("UPDATE files SET excerpt=NULL,hash=NULL WHERE scope_id=?1 AND generation<>?2 AND (excerpt IS NOT NULL OR hash IS NOT NULL)",params![id,generation])?;
        tx.execute("UPDATE scopes SET generation=?1,status=?2,scanned_at=?3,omitted=?4,content=?5 WHERE id=?6",params![generation,snapshot.status,SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64,snapshot.omission_count as i64,content,id])?;
        tx.commit()
    }
    pub fn search(
        &self,
        id: i64,
        query: &str,
        size_sort: bool,
        offset: u32,
    ) -> rusqlite::Result<Page> {
        self.root(id)?;
        if query.len() > 256 {
            return Err(rusqlite::Error::InvalidParameterName(
                "Search is limited to 256 bytes".into(),
            ));
        }
        let phrase = format!("\"{}\"", query.replace('"', "\"\""));
        let filter = "FROM files f JOIN scopes s ON s.id=f.scope_id WHERE s.id=?1 AND f.generation=s.generation AND (?2='' OR instr(lower(f.display),lower(?2))>0 OR f.id IN (SELECT rowid FROM search WHERE search MATCH ?3))";
        let total = self.connection.query_row(
            &format!("SELECT COUNT(*) {filter}"),
            params![id, query, phrase],
            |r| r.get(0),
        )?;
        let order = if size_sort {
            "f.size DESC,f.id"
        } else {
            "f.display COLLATE NOCASE,f.id"
        };
        let files = self.connection.prepare(&format!("SELECT f.id,f.display,f.size,f.modified,substr(f.excerpt,1,400),f.hash IS NOT NULL {filter} ORDER BY {order} LIMIT 100 OFFSET ?4"))?.query_map(params![id,query,phrase,offset.min(100_000)],|r|Ok(FileRow { id:r.get(0)?,path:r.get(1)?,size:r.get(2)?,modified:r.get(3)?,excerpt:r.get(4)?,hashed:r.get(5)? }))?.collect::<rusqlite::Result<_>>()?;
        Ok(Page { files, total })
    }
    /// Changes on this connection, including moves with unchanged byte/count totals.
    pub fn revision(&self) -> u64 {
        self.connection.total_changes()
    }
    pub fn storage_files(&self, id: i64) -> rusqlite::Result<Vec<ScopeFileRecord>> {
        self.root(id)?;
        self.connection.prepare("SELECT f.id,f.path,f.display,f.identity,f.size,f.modified FROM files f JOIN scopes s ON s.id=f.scope_id WHERE s.id=?1 AND f.generation=s.generation ORDER BY f.id")?
            .query_map([id], |r| Ok(ScopeFileRecord {
                id:r.get(0)?,path:path_from_bytes(r.get(1)?),display:r.get(2)?,identity:r.get(3)?,size:r.get::<_,i64>(4)? as u64,modified:r.get(5)?,excerpt:None,hash:None
            }))?.collect()
    }
    pub fn current_files(&self, id: i64) -> rusqlite::Result<Vec<ScopeFileRecord>> {
        self.root(id)?;
        let mut stmt = self.connection.prepare(
            "SELECT f.id, f.path, f.display, f.identity, f.size, f.modified, f.excerpt, f.hash \
             FROM files f JOIN scopes s ON s.id=f.scope_id \
             WHERE s.id=?1 AND f.generation=s.generation \
             ORDER BY f.id",
        )?;
        let rows = stmt.query_map([id], |r| {
            let p_bytes: Vec<u8> = r.get(1)?;
            Ok(ScopeFileRecord {
                id: r.get(0)?,
                path: path_from_bytes(p_bytes),
                display: r.get(2)?,
                identity: r.get(3)?,
                size: r.get::<_, i64>(4)? as u64,
                modified: r.get(5)?,
                excerpt: r.get(6)?,
                hash: r.get(7)?,
            })
        })?;
        rows.collect()
    }
}

fn root_identity(root: &crate::AuthorizedRoot) -> String {
    #[cfg(unix)]
    {
        let (device, inode) = root.identity();
        format!("{device}:{inode}")
    }
    #[cfg(not(unix))]
    {
        root.path().to_string_lossy().into()
    }
}

impl Index {
    /// Verified copies retain their source row and receive a new opaque ID.
    pub fn reconcile_copies(
        &mut self,
        scope: i64,
        copies: &[(PathBuf, PathBuf, String)],
    ) -> rusqlite::Result<()> {
        let tx = self.connection.transaction()?;
        for (source, dest, stamp) in copies {
            let parts: Vec<_> = stamp.split(':').collect();
            if parts.len() != 7 {
                return Err(rusqlite::Error::InvalidParameterName(
                    "Invalid copy evidence".into(),
                ));
            }
            let identity = format!("{}:{}", parts[0], parts[1]);
            let modified = parts[3]
                .parse::<i64>()
                .map_err(|_| rusqlite::Error::InvalidParameterName("Invalid copy time".into()))?;
            tx.execute("INSERT INTO files(scope_id,path,display,identity,fingerprint,size,modified,excerpt,hash,generation) SELECT scope_id,?1,?2,?3,?4,size,?5,excerpt,hash,generation FROM files WHERE scope_id=?6 AND path=?7",params![path_bytes(dest),dest.to_string_lossy(),identity,stamp,modified,scope,path_bytes(source)])?;
        }
        tx.commit()
    }
    /// Reconcile only journal-verified changes; avoid a full blocking rescan after execution.
    /// Re-paths every indexed file below a moved or renamed folder.
    pub fn rename_tree(&mut self, scope: i64, from: &Path, to: &Path) -> rusqlite::Result<usize> {
        let mut prefix = path_bytes(from);
        prefix.push(b'/');
        let tx = self.connection.transaction()?;
        let rows: Vec<(i64, Vec<u8>)> = tx
            .prepare("SELECT id,path FROM files WHERE scope_id=?1 AND length(path)>?2 AND substr(path,1,?2)=?3")?
            .query_map(params![scope, prefix.len() as i64, prefix], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut moved = 0;
        for (id, old) in rows {
            let new = to.join(path_from_bytes(old[prefix.len()..].to_vec()));
            tx.execute(
                "UPDATE files SET path=?1,display=?2 WHERE id=?3",
                params![path_bytes(&new), new.to_string_lossy(), id],
            )?;
            moved += 1;
        }
        tx.commit()?;
        Ok(moved)
    }
    /// Forgets a trashed folder and every indexed path below it.
    pub fn remove_tree(&mut self, scope: i64, dir: &Path) -> rusqlite::Result<usize> {
        let mut prefix = path_bytes(dir);
        let exact = prefix.clone();
        prefix.push(b'/');
        let tx = self.connection.transaction()?;
        let removed = tx.execute(
            "DELETE FROM files WHERE scope_id=?1 AND (path=?2 OR (length(path)>?3 AND substr(path,1,?3)=?4))",
            params![scope, exact, prefix.len() as i64, prefix],
        )?;
        tx.commit()?;
        Ok(removed)
    }
    pub fn reconcile_verified(
        &mut self,
        scope: i64,
        changes: &[(PathBuf, Option<(PathBuf, String)>)],
    ) -> rusqlite::Result<()> {
        let tx = self.connection.transaction()?;
        for (source, destination) in changes {
            if let Some((dest, stamp)) = destination {
                tx.execute("UPDATE files SET path=?1,display=?2,fingerprint=?3 WHERE scope_id=?4 AND path=?5",params![path_bytes(dest),dest.to_string_lossy(),stamp,scope,path_bytes(source)])?;
            } else {
                tx.execute(
                    "DELETE FROM files WHERE scope_id=?1 AND path=?2",
                    params![scope, path_bytes(source)],
                )?;
            }
        }
        tx.commit()
    }
}
