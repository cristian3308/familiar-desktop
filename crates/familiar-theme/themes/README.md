# Temas

Cada archivo `.toml` es un tema. Al arrancar, Familiar Core copia estos temas
a `%APPDATA%\FamiliarCore\themes\`; edita la copia y los cambios se aplican
en caliente al guardar.

| Clave | Valores |
|---|---|
| `colors.*` | `#RRGGBB` o `#RRGGBBAA` |
| `metrics.dock_icon_size` | 24 – 128 |
| `metrics.dock_magnification` | 1.0 (sin efecto) – 2.5 |
| `metrics.corner_radius` | 0 – 40 |
| `metrics.top_bar_height` | 22 – 48 |
| `metrics.blur_type` | `None`, `Mica`, `Acrylic` (solo en el build `--features winit`; el build por defecto usa el tinte translúcido) |
| `cursor.custom_scheme` | `true` aplica los `.cur`/`.ani` de `cursor.scheme_dir` (por defecto `cursors/` junto al tema) |
| `cursor.ripple_on_click` | onda al hacer clic |
| `cursor.shake_to_find` | halo al agitar el ratón |

Nombres de cursor reconocidos: `arrow`, `ibeam`, `wait`, `cross`, `up`,
`size_nwse`, `size_nesw`, `size_we`, `size_ns`, `size_all`, `no`, `hand`,
`app_starting`, `help`.
