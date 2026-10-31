//! UI language: Español (source), English, Français and Русский.
//!
//! Texts written in `.slint` files are wrapped in `@tr(...)` and translated
//! by Slint from the `.po` files bundled at build time (`lang/<locale>/`).
//! The few texts Rust puts on screen (alerts, launcher hints, Calm rules)
//! are translated here with [`t`]; Spanish is the key and the fallback.

use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    Es,
    En,
    Fr,
    Ru,
}

impl Lang {
    pub const ALL: [Lang; 4] = [Lang::Es, Lang::En, Lang::Fr, Lang::Ru];

    /// Code stored in `config.toml` and shown in the top bar.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Es => "es",
            Lang::En => "en",
            Lang::Fr => "fr",
            Lang::Ru => "ru",
        }
    }

    pub fn from_code(code: &str) -> Self {
        Self::ALL.into_iter().find(|l| l.code().eq_ignore_ascii_case(code.trim())).unwrap_or_default()
    }

    /// The language's own name.
    pub fn name(self) -> &'static str {
        match self {
            Lang::Es => "Español",
            Lang::En => "English",
            Lang::Fr => "Français",
            Lang::Ru => "Русский",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|l| *l == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Folder name under `lang/`. Spanish is the source language (empty);
    /// English needs a region because Slint treats a bare "en" as "source".
    pub fn slint_locale(self) -> &'static str {
        match self {
            Lang::Es => "",
            Lang::En => "en-US",
            Lang::Fr => "fr",
            Lang::Ru => "ru",
        }
    }
}

thread_local! {
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::Es) };
}

pub fn current() -> Lang {
    CURRENT.with(|c| c.get())
}

/// Switches Rust-side texts and Slint's bundled translations. Call it
/// after the first window exists.
pub fn set(lang: Lang) {
    CURRENT.with(|c| c.set(lang));
    #[cfg(windows)]
    if let Err(e) = slint::select_bundled_translation(lang.slint_locale()) {
        log::warn!("language {}: {e:?}", lang.code());
    }
}

/// `es` translated to the current language.
pub fn t(es: &'static str) -> &'static str {
    translate(current(), es)
}

/// Rows: Spanish, English, French, Russian.
const TABLE: &[[&str; 4]] = &[
    ["Seguir", "Keep running", "Continuer", "Продолжать"],
    ["Pausar visualizador", "Pause visualizer", "Pause du visualiseur", "Пауза визуализатора"],
    ["Pausar escritorio", "Pause desktop", "Pause du bureau", "Пауза рабочего стола"],
    ["Conectado", "Connected", "Connecté", "Подключено"],
    ["Desconectado", "Disconnected", "Déconnecté", "Отключено"],
    ["Salida", "Output", "Sortie", "Вывод"],
    [
        "Focus terminado · tómate un descanso",
        "Focus done · take a break",
        "Focus terminé · faites une pause",
        "Фокус завершён · отдохните",
    ],
    ["Calculadora", "Calculator", "Calculatrice", "Калькулятор"],
    ["COPIAR ↵", "COPY ↵", "COPIER ↵", "КОПИРОВАТЬ ↵"],
    ["ABRIR ↵", "OPEN ↵", "OUVRIR ↵", "ОТКРЫТЬ ↵"],
    ["Carpeta reciente", "Recent folder", "Dossier récent", "Недавняя папка"],
    ["Archivo reciente", "Recent file", "Fichier récent", "Недавний файл"],
    ["Escritorio", "Desktop", "Bureau", "Рабочий стол"],
    // Weather (familiar_sys::weather::Sky::describe_es).
    ["Despejado", "Clear", "Dégagé", "Ясно"],
    ["Parcialmente nublado", "Partly cloudy", "Partiellement nuageux", "Переменная облачность"],
    ["Nublado", "Cloudy", "Nuageux", "Облачно"],
    ["Niebla", "Fog", "Brouillard", "Туман"],
    ["Lluvia", "Rain", "Pluie", "Дождь"],
    ["Nieve", "Snow", "Neige", "Снег"],
    ["Tormenta", "Storm", "Orage", "Гроза"],
    // Dock menu.
    ["Abrir", "Open", "Ouvrir", "Открыть"],
    ["Nueva ventana", "New window", "Nouvelle fenêtre", "Новое окно"],
    ["Cerrar todas las ventanas", "Close all windows", "Fermer toutes les fenêtres", "Закрыть все окна"],
    ["Fijar en el Dock", "Keep in Dock", "Garder dans le Dock", "Закрепить в Доке"],
    ["Quitar del Dock", "Remove from Dock", "Retirer du Dock", "Убрать из Дока"],
    ["Mostrar en el Explorador", "Show in Explorer", "Afficher dans l'Explorateur", "Показать в Проводнике"],
    ["Vaciar la papelera", "Empty Trash", "Vider la corbeille", "Очистить корзину"],
    ["Abrir la papelera", "Open Trash", "Ouvrir la corbeille", "Открыть корзину"],
    ["Ajustes del Dock", "Dock settings", "Réglages du Dock", "Настройки Дока"],
];

/// Locale for Windows date formatting; `None` keeps the user's own (for
/// Spanish, so their regional settings still apply).
pub fn locale() -> Option<&'static str> {
    match current() {
        Lang::Es => None,
        Lang::En => Some("en-US"),
        Lang::Fr => Some("fr-FR"),
        Lang::Ru => Some("ru-RU"),
    }
}

/// Long date for the lock face ("viernes, 30 de octubre").
pub fn long_date_picture() -> &'static str {
    match current() {
        Lang::Es => "dddd, d 'de' MMMM",
        Lang::En => "dddd, MMMM d",
        Lang::Fr => "dddd d MMMM",
        Lang::Ru => "dddd, d MMMM",
    }
}

/// Calendar column headers, Monday first.
pub fn weekday_initials() -> [&'static str; 7] {
    match current() {
        Lang::Es => ["L", "M", "X", "J", "V", "S", "D"],
        Lang::En => ["M", "T", "W", "T", "F", "S", "S"],
        Lang::Fr => ["L", "M", "M", "J", "V", "S", "D"],
        Lang::Ru => ["П", "В", "С", "Ч", "П", "С", "В"],
    }
}

/// A date in the current language.
#[cfg(windows)]
pub fn date(picture: &str) -> String {
    familiar_sys::clock::date_string_in(locale(), picture)
}

fn translate(lang: Lang, es: &'static str) -> &'static str {
    let col = Lang::ALL.iter().position(|l| *l == lang).unwrap_or(0);
    TABLE.iter().find(|row| row[0] == es).map_or(es, |row| row[col])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_cycle() {
        for l in Lang::ALL {
            assert_eq!(Lang::from_code(l.code()), l);
        }
        assert_eq!(Lang::from_code("xx"), Lang::Es);
        assert_eq!(Lang::Ru.next(), Lang::Es);
    }

    #[test]
    fn rust_texts_translate_with_spanish_fallback() {
        assert_eq!(translate(Lang::En, "Seguir"), "Keep running");
        assert_eq!(translate(Lang::Ru, "Calculadora"), "Калькулятор");
        assert_eq!(translate(Lang::Fr, "no existe"), "no existe");
        assert_eq!(translate(Lang::Es, "Salida"), "Salida");
        for row in TABLE {
            assert!(row.iter().all(|s| !s.is_empty()));
        }
    }

    /// Every `@tr("...")` text in the UI has an entry in every `.po` file.
    #[test]
    fn every_ui_text_is_translated() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(root.join("ui")).unwrap() {
            let src = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            for part in src.split("@tr(\"").skip(1) {
                let id = part.split("\")").next().unwrap();
                ids.push(id.to_string());
            }
        }
        assert!(ids.len() > 100, "found only {} texts", ids.len());
        for locale in ["en-US", "fr", "ru"] {
            let po =
                std::fs::read_to_string(root.join("lang").join(locale).join("LC_MESSAGES/familiar-core.po")).unwrap();
            for id in &ids {
                assert!(po.contains(&format!("msgid \"{id}\"")), "{locale}: missing {id:?}");
            }
        }
    }
}
