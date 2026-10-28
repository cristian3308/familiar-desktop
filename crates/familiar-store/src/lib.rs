//! Local persistence (`familiar_state.db`).
//!
//! The launcher keeps the whole app index in memory ([`AppSearch`]) and only
//! writes back launch statistics, so a keystroke never touches the disk.

mod search;

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

pub use rusqlite::Error;
pub use search::{AppSearch, SearchHit, fuzzy_score};

const SCHEMA_VERSION: i32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockItem {
    pub id: i64,
    pub app_id: String,
    pub display_name: String,
    pub executable_path: String,
    pub icon_cache_path: String,
    pub position_index: i64,
    pub is_pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewDockItem<'a> {
    pub app_id: &'a str,
    pub display_name: &'a str,
    pub executable_path: &'a str,
    pub icon_cache_path: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WidgetLayout {
    pub id: String,
    pub target_screen: String,
    pub grid_x: i32,
    pub grid_y: i32,
    pub width_units: i32,
    pub height_units: i32,
    pub custom_config_json: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppEntry {
    pub id: i64,
    pub name: String,
    pub launch_target: String,
    pub executable_path: String,
    pub icon_cache_path: String,
    pub category: Option<String>,
    /// Unix seconds.
    pub last_launched: Option<i64>,
    pub launch_count: i64,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, Error> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, Error> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, Error> {
        // WAL keeps launch-count writes from blocking reads; a small cache
        // keeps SQLite's own footprint well under a megabyte.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "cache_size", -256)?;
        let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < SCHEMA_VERSION {
            conn.execute_batch(include_str!("schema.sql"))?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(Self { conn })
    }

    // ----- dock -----------------------------------------------------------

    pub fn dock_items(&self) -> Result<Vec<DockItem>, Error> {
        let mut st = self.conn.prepare_cached(
            "SELECT id, app_id, display_name, executable_path, icon_cache_path, position_index, is_pinned
             FROM dock_items WHERE is_pinned = 1 ORDER BY position_index, id",
        )?;
        st.query_map([], |r| {
            Ok(DockItem {
                id: r.get(0)?,
                app_id: r.get(1)?,
                display_name: r.get(2)?,
                executable_path: r.get(3)?,
                icon_cache_path: r.get(4)?,
                position_index: r.get(5)?,
                is_pinned: r.get(6)?,
            })
        })?
        .collect()
    }

    /// Pins an app at the end of the dock (or re-pins it if it was unpinned).
    pub fn pin(&self, item: &NewDockItem<'_>) -> Result<i64, Error> {
        let next: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(position_index) + 1, 0) FROM dock_items WHERE is_pinned = 1",
            [],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO dock_items (app_id, display_name, executable_path, icon_cache_path, position_index, is_pinned)
             VALUES (?1, ?2, ?3, ?4, ?5, 1)
             ON CONFLICT(app_id) DO UPDATE SET
                display_name = excluded.display_name,
                executable_path = excluded.executable_path,
                icon_cache_path = excluded.icon_cache_path,
                position_index = CASE WHEN dock_items.is_pinned THEN dock_items.position_index ELSE excluded.position_index END,
                is_pinned = 1",
            params![item.app_id, item.display_name, item.executable_path, item.icon_cache_path, next],
        )?;
        self.conn.query_row("SELECT id FROM dock_items WHERE app_id = ?1", [item.app_id], |r| r.get(0))
    }

    pub fn unpin(&self, app_id: &str) -> Result<bool, Error> {
        Ok(self.conn.execute("DELETE FROM dock_items WHERE app_id = ?1", [app_id])? > 0)
    }

    pub fn is_pinned(&self, app_id: &str) -> Result<bool, Error> {
        Ok(self
            .conn
            .query_row("SELECT is_pinned FROM dock_items WHERE app_id = ?1", [app_id], |r| r.get::<_, bool>(0))
            .optional()?
            .unwrap_or(false))
    }

    /// Rewrites positions to follow `app_ids`; ids not listed keep their
    /// relative order after the listed ones.
    pub fn reorder_dock(&mut self, app_ids: &[&str]) -> Result<(), Error> {
        let current = self.dock_items()?;
        let tx = self.conn.transaction()?;
        {
            let mut st = tx.prepare_cached("UPDATE dock_items SET position_index = ?1 WHERE app_id = ?2")?;
            let mut pos = 0i64;
            for id in app_ids {
                if st.execute(params![pos, id])? > 0 {
                    pos += 1;
                }
            }
            for item in current.iter().filter(|i| !app_ids.contains(&i.app_id.as_str())) {
                st.execute(params![pos, item.app_id])?;
                pos += 1;
            }
        }
        tx.commit()
    }

    pub fn set_dock_icon(&self, app_id: &str, icon_cache_path: &str) -> Result<(), Error> {
        self.conn.execute(
            "UPDATE dock_items SET icon_cache_path = ?1 WHERE app_id = ?2",
            params![icon_cache_path, app_id],
        )?;
        Ok(())
    }

    // ----- widgets --------------------------------------------------------

    pub fn widget_layouts(&self, target_screen: &str) -> Result<Vec<WidgetLayout>, Error> {
        let mut st = self.conn.prepare_cached(
            "SELECT id, target_screen, grid_x, grid_y, width_units, height_units, custom_config_json
             FROM widget_layouts WHERE target_screen = ?1 ORDER BY id",
        )?;
        st.query_map([target_screen], |r| {
            Ok(WidgetLayout {
                id: r.get(0)?,
                target_screen: r.get(1)?,
                grid_x: r.get(2)?,
                grid_y: r.get(3)?,
                width_units: r.get(4)?,
                height_units: r.get(5)?,
                custom_config_json: r.get(6)?,
            })
        })?
        .collect()
    }

    pub fn upsert_widget(&self, w: &WidgetLayout) -> Result<(), Error> {
        self.conn.execute(
            "INSERT INTO widget_layouts (id, target_screen, grid_x, grid_y, width_units, height_units, custom_config_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                target_screen = excluded.target_screen, grid_x = excluded.grid_x, grid_y = excluded.grid_y,
                width_units = excluded.width_units, height_units = excluded.height_units,
                custom_config_json = excluded.custom_config_json",
            params![w.id, w.target_screen, w.grid_x, w.grid_y, w.width_units, w.height_units, w.custom_config_json],
        )?;
        Ok(())
    }

    // ----- app index ------------------------------------------------------

    pub fn apps(&self) -> Result<Vec<AppEntry>, Error> {
        let mut st = self.conn.prepare_cached(
            "SELECT id, name, launch_target, executable_path, icon_cache_path, category,
                    CAST(strftime('%s', last_launched) AS INTEGER), launch_count
             FROM app_index ORDER BY name COLLATE NOCASE",
        )?;
        st.query_map([], |r| {
            Ok(AppEntry {
                id: r.get(0)?,
                name: r.get(1)?,
                launch_target: r.get(2)?,
                executable_path: r.get(3)?,
                icon_cache_path: r.get(4)?,
                category: r.get(5)?,
                last_launched: r.get(6)?,
                launch_count: r.get(7)?,
            })
        })?
        .collect()
    }

    /// Replaces the index with a fresh scan while keeping launch statistics
    /// for apps that are still installed.
    pub fn replace_apps(&mut self, apps: &[AppEntry]) -> Result<(), Error> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE app_index SET seen = 0", [])?;
        {
            let mut st = tx.prepare_cached(
                "INSERT INTO app_index (name, launch_target, executable_path, icon_cache_path, category, seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1)
                 ON CONFLICT(launch_target) DO UPDATE SET
                    name = excluded.name,
                    executable_path = excluded.executable_path,
                    icon_cache_path = CASE WHEN excluded.icon_cache_path = '' THEN app_index.icon_cache_path
                                           ELSE excluded.icon_cache_path END,
                    category = excluded.category,
                    seen = 1",
            )?;
            for a in apps {
                st.execute(params![a.name, a.launch_target, a.executable_path, a.icon_cache_path, a.category])?;
            }
        }
        tx.execute("DELETE FROM app_index WHERE seen = 0", [])?;
        tx.commit()
    }

    pub fn set_app_icon(&self, launch_target: &str, icon_cache_path: &str) -> Result<(), Error> {
        self.conn.execute(
            "UPDATE app_index SET icon_cache_path = ?1 WHERE launch_target = ?2",
            params![icon_cache_path, launch_target],
        )?;
        Ok(())
    }

    pub fn record_launch(&self, launch_target: &str) -> Result<(), Error> {
        self.conn.execute(
            "UPDATE app_index SET launch_count = launch_count + 1, last_launched = datetime('now')
             WHERE launch_target = ?1",
            [launch_target],
        )?;
        Ok(())
    }

    pub fn app_by_target(&self, launch_target: &str) -> Result<Option<AppEntry>, Error> {
        Ok(self.apps()?.into_iter().find(|a| a.launch_target == launch_target))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item<'a>(id: &'a str) -> NewDockItem<'a> {
        NewDockItem { app_id: id, display_name: id, executable_path: "", icon_cache_path: "" }
    }

    fn app(name: &str, target: &str) -> AppEntry {
        AppEntry { name: name.into(), launch_target: target.into(), ..Default::default() }
    }

    #[test]
    fn pin_order_unpin_reorder() {
        let mut s = Store::open_in_memory().unwrap();
        for id in ["a", "b", "c"] {
            s.pin(&item(id)).unwrap();
        }
        // Pinning again does not move or duplicate it.
        s.pin(&item("a")).unwrap();
        let ids = |s: &Store| s.dock_items().unwrap().into_iter().map(|i| i.app_id).collect::<Vec<_>>();
        assert_eq!(ids(&s), ["a", "b", "c"]);
        assert!(s.unpin("b").unwrap());
        assert!(!s.is_pinned("b").unwrap());
        s.pin(&item("d")).unwrap();
        s.reorder_dock(&["d", "a"]).unwrap();
        assert_eq!(ids(&s), ["d", "a", "c"]);
    }

    #[test]
    fn widgets_upsert() {
        let s = Store::open_in_memory().unwrap();
        let mut w = WidgetLayout {
            id: "signal_01".into(),
            target_screen: "desktop".into(),
            grid_x: 1,
            grid_y: 2,
            width_units: 2,
            height_units: 2,
            custom_config_json: None,
        };
        s.upsert_widget(&w).unwrap();
        w.grid_x = 5;
        s.upsert_widget(&w).unwrap();
        assert_eq!(s.widget_layouts("desktop").unwrap(), vec![w]);
        assert!(s.widget_layouts("lockscreen").unwrap().is_empty());
    }

    #[test]
    fn replace_apps_keeps_stats_and_drops_uninstalled() {
        let mut s = Store::open_in_memory().unwrap();
        s.replace_apps(&[app("Code", "code"), app("Paint", "paint")]).unwrap();
        s.record_launch("code").unwrap();
        s.record_launch("code").unwrap();
        s.replace_apps(&[app("Visual Studio Code", "code"), app("Terminal", "wt")]).unwrap();
        let apps = s.apps().unwrap();
        assert_eq!(apps.len(), 2);
        let code = apps.iter().find(|a| a.launch_target == "code").unwrap();
        assert_eq!(code.launch_count, 2);
        assert_eq!(code.name, "Visual Studio Code");
        assert!(code.last_launched.is_some());
        assert!(apps.iter().all(|a| a.launch_target != "paint"));
    }

    #[test]
    fn reopening_a_file_keeps_data() {
        let dir = std::env::temp_dir().join(format!("familiar-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.db");
        {
            let s = Store::open(&path).unwrap();
            s.pin(&item("x")).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert!(s.is_pinned("x").unwrap());
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }
}
