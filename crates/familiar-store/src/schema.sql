-- 1. Apps pinned to the dock and their order
CREATE TABLE IF NOT EXISTS dock_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    app_id TEXT NOT NULL UNIQUE,          -- AppUserModelID or .exe path (AppsFolder parsing name)
    display_name TEXT NOT NULL,
    executable_path TEXT NOT NULL,        -- resolved .exe, '' when unknown (packaged apps)
    icon_cache_path TEXT NOT NULL,        -- 256x256 PNG extracted with IShellItemImageFactory
    position_index INTEGER NOT NULL,
    is_pinned BOOLEAN NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_dock_pos ON dock_items(position_index);

-- 2. Widget positions on the desktop and the lock screen
CREATE TABLE IF NOT EXISTS widget_layouts (
    id TEXT PRIMARY KEY,                  -- 'signal_01', 'clock_grid', ...
    target_screen TEXT NOT NULL,          -- 'desktop' | 'lockscreen'
    grid_x INTEGER NOT NULL,
    grid_y INTEGER NOT NULL,
    width_units INTEGER NOT NULL,
    height_units INTEGER NOT NULL,
    custom_config_json TEXT
);

-- 3. Launcher application cache
CREATE TABLE IF NOT EXISTS app_index (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    launch_target TEXT NOT NULL UNIQUE,
    executable_path TEXT NOT NULL DEFAULT '',
    icon_cache_path TEXT NOT NULL DEFAULT '',
    category TEXT,
    last_launched DATETIME,
    launch_count INTEGER NOT NULL DEFAULT 0,
    seen INTEGER NOT NULL DEFAULT 1       -- scratch flag used by replace_apps
);
CREATE INDEX IF NOT EXISTS idx_app_search ON app_index(name, launch_count DESC);
