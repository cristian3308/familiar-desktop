"""Source of the bundled UI translations.

Spanish is the source language (the strings written in `ui/*.slint` with
`@tr`). Run `python lang/translations.py` after adding or changing texts to
regenerate `lang/<locale>/LC_MESSAGES/familiar-core.po`; the build bundles
them into the executable.
"""
from pathlib import Path

# Spanish source -> (English, French, Russian)
T = {
    "CONTROL CENTER": ("CONTROL CENTER", "CENTRE DE CONTRÔLE", "ПУНКТ УПРАВЛЕНИЯ"),
    "Wi-Fi": ("Wi-Fi", "Wi-Fi", "Wi-Fi"),
    "No disponible": ("Unavailable", "Indisponible", "Недоступно"),
    "Activado": ("On", "Activé", "Вкл."),
    "Desactivado": ("Off", "Désactivé", "Выкл."),
    "Bluetooth": ("Bluetooth", "Bluetooth", "Bluetooth"),
    "Pantalla": ("Display", "Écran", "Экран"),
    "Sonido": ("Sound", "Son", "Звук"),
    "Silencio": ("Mute", "Muet", "Без звука"),
    "Apariencia": ("Appearance", "Apparence", "Оформление"),
    "Claro": ("Light", "Clair", "Светлая"),
    "Oscuro": ("Dark", "Sombre", "Тёмная"),
    "Bloquear": ("Lock", "Verrouiller", "Блокировка"),
    "Familiar": ("Familiar", "Familiar", "Familiar"),
    "Configuración del sistema": ("System Settings", "Réglages système", "Параметры системы"),
    " VENTANA": (" WINDOW", " FENÊTRE", " ОКНО"),
    " VENTANAS": (" WINDOWS", " FENÊTRES", " ОКНА"),
    "REPRODUCIENDO": ("PLAYING", "LECTURE", "ИГРАЕТ"),
    "EN PAUSA": ("PAUSED", "EN PAUSE", "ПАУЗА"),
    "SIN MÚSICA": ("NO MUSIC", "PAS DE MUSIQUE", "НЕТ МУЗЫКИ"),
    "FOCUS ": ("FOCUS ", "FOCUS ", "ФОКУС "),
    "FOCUS": ("FOCUS", "FOCUS", "ФОКУС"),
    "Nada reproduciéndose": ("Nothing playing", "Rien en lecture", "Ничего не играет"),
    "BUSCAR": ("SEARCH", "RECHERCHE", "ПОИСК"),
    "= 2+2 CALCULA": ("= 2+2 CALCULATES", "= 2+2 CALCULE", "= 2+2 СЧИТАЕТ"),
    "Buscar en recientes…": ("Search recent…", "Rechercher dans les récents…", "Поиск в недавних…"),
    "Filtrar apps…": ("Filter apps…", "Filtrer les apps…", "Фильтр приложений…"),
    "Abrir una app…": ("Open an app…", "Ouvrir une app…", "Открыть приложение…"),
    "APPS": ("APPS", "APPS", "ПРИЛОЖЕНИЯ"),
    "RECIENTES": ("RECENT", "RÉCENTS", "НЕДАВНИЕ"),
    "TAB CAMBIA": ("TAB SWITCHES", "TAB CHANGE", "TAB — ВКЛАДКИ"),
    "SIN ARCHIVOS RECIENTES": ("NO RECENT FILES", "AUCUN FICHIER RÉCENT", "НЕТ НЕДАВНИХ ФАЙЛОВ"),
    "SIN RESULTADOS": ("NO RESULTS", "AUCUN RÉSULTAT", "НИЧЕГО НЕ НАЙДЕНО"),
    "↑↓ NAVEGAR": ("↑↓ NAVIGATE", "↑↓ NAVIGUER", "↑↓ ВЫБОР"),
    "↵ ABRIR": ("↵ OPEN", "↵ OUVRIR", "↵ ОТКРЫТЬ"),
    "ESC CERRAR": ("ESC CLOSE", "ÉCHAP FERMER", "ESC ЗАКРЫТЬ"),
    "FAMILIAR": ("FAMILIAR", "FAMILIAR", "FAMILIAR"),
    "Haz clic o pulsa una tecla para entrar": ("Click or press a key to enter", "Cliquez ou appuyez sur une touche pour entrer", "Щёлкните или нажмите клавишу, чтобы войти"),
    "CARGANDO · ": ("CHARGING · ", "EN CHARGE · ", "ЗАРЯДКА · "),
    "BATERÍA · ": ("BATTERY · ", "BATTERIE · ", "БАТАРЕЯ · "),
    "WIN+L BLOQUEA CON CONTRASEÑA": ("WIN+L LOCKS WITH PASSWORD", "WIN+L VERROUILLE AVEC MOT DE PASSE", "WIN+L — БЛОКИРОВКА С ПАРОЛЕМ"),
    "Ajustes de Familiar": ("Familiar Settings", "Réglages de Familiar", "Настройки Familiar"),
    "Dock": ("Dock", "Dock", "Док"),
    "Widgets": ("Widgets", "Widgets", "Виджеты"),
    "General": ("General", "Général", "Общие"),
    "Calma e idioma": ("Calm & language", "Calme et langue", "Покой и язык"),
    "TEMA": ("THEME", "THÈME", "ТЕМА"),
    "Aa": ("Aa", "Aa", "Aa"),
    "Surprise me": ("Surprise me", "Surprends-moi", "Удиви меня"),
    "Tema desde el fondo": ("Theme from wallpaper", "Thème du fond d'écran", "Тема из обоев"),
    "COLOR DE ACENTO": ("ACCENT COLOUR", "COULEUR D'ACCENT", "АКЦЕНТНЫЙ ЦВЕТ"),
    "BARRA SUPERIOR": ("TOP BAR", "BARRE SUPÉRIEURE", "ВЕРХНЯЯ ПАНЕЛЬ"),
    "Opacidad": ("Opacity", "Opacité", "Непрозрачность"),
    "Tamaño de los iconos": ("Icon size", "Taille des icônes", "Размер значков"),
    " px": (" px", " px", " пикс."),
    "Ampliación": ("Magnification", "Agrandissement", "Увеличение"),
    "Iconos estilo macOS": ("macOS-style icons", "Icônes style macOS", "Значки в стиле macOS"),
    "Explorador, Configuración y Terminal": ("Explorer, Settings and Terminal", "Explorateur, Paramètres et Terminal", "Проводник, Параметры и Терминал"),
    "Icono de Archivos": ("Files icon", "Icône Fichiers", "Значок «Файлы»"),
    "Classic, Dot F, Mono, Theme o Glass": ("Classic, Dot F, Mono, Theme or Glass", "Classic, Dot F, Mono, Theme ou Glass", "Classic, Dot F, Mono, Theme или Glass"),
    "Ventanas al pasar el ratón": ("Windows on hover", "Fenêtres au survol", "Окна при наведении"),
    "Lista las ventanas abiertas de la app": ("Lists the app's open windows", "Liste les fenêtres ouvertes de l'app", "Список открытых окон приложения"),
    "Mostrar apps abiertas": ("Show open apps", "Afficher les apps ouvertes", "Показывать открытые приложения"),
    "Calendario": ("Calendar", "Calendrier", "Календарь"),
    "Clima": ("Weather", "Météo", "Погода"),
    "Usa internet (Open-Meteo)": ("Uses the internet (Open-Meteo)", "Utilise Internet (Open-Meteo)", "Использует интернет (Open-Meteo)"),
    "Sistema": ("System", "Système", "Система"),
    "CPU, memoria y batería": ("CPU, memory and battery", "Processeur, mémoire et batterie", "ЦП, память и батарея"),
    "SIGNAL/01": ("SIGNAL/01", "SIGNAL/01", "SIGNAL/01"),
    "Matriz de puntos: pulsa el modo para cambiarlo": ("Dot matrix: press the mode to change it", "Matrice de points : touchez le mode pour le changer", "Точечная матрица: нажмите режим, чтобы сменить"),
    "Reloj de puntos": ("Dot clock", "Horloge à points", "Точечные часы"),
    "Visualizador": ("Visualizer", "Visualiseur", "Визуализатор"),
    "Seis escenas; toca el widget para cambiar": ("Six scenes; tap the widget to switch", "Six scènes ; touchez le widget pour changer", "Шесть сцен; нажмите на виджет для смены"),
    "Reloj analógico": ("Analog clock", "Horloge analogique", "Аналоговые часы"),
    "Música": ("Music", "Musique", "Музыка"),
    "Carátula y controles": ("Artwork and controls", "Pochette et commandes", "Обложка и управление"),
    "CIUDAD DEL CLIMA": ("WEATHER CITY", "VILLE DE LA MÉTÉO", "ГОРОД ДЛЯ ПОГОДЫ"),
    "Pulsa Intro para guardar la ciudad.": ("Press Enter to save the city.", "Appuyez sur Entrée pour enregistrer la ville.", "Нажмите Enter, чтобы сохранить город."),
    "CALM BY DESIGN": ("CALM BY DESIGN", "CALME PAR CONCEPTION", "СПОКОЙСТВИЕ ПО УМОЛЧАНИЮ"),
    "Ventana en primer plano": ("Focused window", "Fenêtre au premier plan", "Активное окно"),
    "Ventana maximizada": ("Maximised window", "Fenêtre agrandie", "Развёрнутое окно"),
    "Pantalla completa o juego": ("Full screen or game", "Plein écran ou jeu", "Полный экран или игра"),
    "Portátil con batería": ("Laptop on battery", "Portable sur batterie", "Ноутбук от батареи"),
    "Se aplica la regla más estricta de las situaciones activas.": ("The strictest rule of the active situations applies.", "La règle la plus stricte des situations actives s'applique.", "Действует самое строгое правило из активных ситуаций."),
    "IDIOMA": ("LANGUAGE", "LANGUE", "ЯЗЫК"),
    "Idioma de la interfaz": ("Interface language", "Langue de l'interface", "Язык интерфейса"),
    "También desde la barra superior": ("Also from the top bar", "Aussi depuis la barre supérieure", "Также из верхней панели"),
    "Ocultar la barra de tareas de Windows": ("Hide the Windows taskbar", "Masquer la barre des tâches Windows", "Скрывать панель задач Windows"),
    "Animación al abrir y minimizar": ("Animate opening and minimising", "Animation à l'ouverture et à la réduction", "Анимация открытия и сворачивания"),
    "Las ventanas salen del Dock y vuelven a él": ("Windows grow from the Dock and return to it", "Les fenêtres sortent du Dock et y retournent", "Окна вылетают из Дока и возвращаются в него"),
    "Efecto genio al minimizar": ("Genie effect when minimising", "Effet génie à la réduction", "Эффект «джинн» при сворачивании"),
    "Apagado: la ventana se encoge suavemente": ("Off: the window shrinks smoothly", "Désactivé : la fenêtre rétrécit en douceur", "Выкл.: окно плавно уменьшается"),
    "Pantalla de bienvenida al iniciar": ("Welcome screen at start-up", "Écran d'accueil au démarrage", "Экран приветствия при запуске"),
    "Splash y pantalla de bloqueo (solo visual)": ("Splash and lock screen (visual only)", "Écran de démarrage et de verrouillage (visuel)", "Заставка и экран блокировки (только вид)"),
    "Alt + W cambia el fondo": ("Alt + W changes the wallpaper", "Alt + W change le fond d'écran", "Alt + W меняет обои"),
    "Recorre la carpeta del fondo actual (al reiniciar)": ("Cycles the current wallpaper's folder (after restart)", "Parcourt le dossier du fond actuel (après redémarrage)", "Перебирает папку текущих обоев (после перезапуска)"),
    "Abrir al iniciar Windows": ("Open at Windows start-up", "Ouvrir au démarrage de Windows", "Запускать вместе с Windows"),
    "Reiniciar Familiar": ("Restart Familiar", "Redémarrer Familiar", "Перезапустить Familiar"),
    "Salir de Familiar": ("Quit Familiar", "Quitter Familiar", "Выйти из Familiar"),
    "Algunos cambios se aplican al reiniciar.": ("Some changes apply after a restart.", "Certains changements s'appliquent au redémarrage.", "Некоторые изменения вступят в силу после перезапуска."),
    "Reiniciar ahora": ("Restart now", "Redémarrer maintenant", "Перезапустить сейчас"),
    "Escritorio": ("Desktop", "Bureau", "Рабочий стол"),
    "Cargando…": ("Loading…", "Chargement…", "Загрузка…"),
    "CPU": ("CPU", "CPU", "ЦП"),
    "MEMORIA": ("MEMORY", "MÉMOIRE", "ПАМЯТЬ"),
    "BATERÍA": ("BATTERY", "BATTERIE", "БАТАРЕЯ"),
    "AC": ("AC", "SECTEUR", "СЕТЬ"),
    "cargando": ("charging", "en charge", "заряжается"),
    "MÚSICA": ("MUSIC", "MUSIQUE", "МУЗЫКА"),
}

LOCALES = ("en-US", "fr", "ru")


def escape(s):
    return s.replace('\\', '\\\\').replace('"', '\\"')


def main():
    here = Path(__file__).parent
    for col, locale in enumerate(LOCALES):
        lines = [
            'msgid ""',
            'msgstr ""',
            '"Content-Type: text/plain; charset=UTF-8\\n"',
            f'"Language: {locale}\\n"',
            '',
        ]
        for es, tr in T.items():
            lines += [f'msgid "{escape(es)}"', f'msgstr "{escape(tr[col])}"', '']
        out = here / locale / 'LC_MESSAGES' / 'familiar-core.po'
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text('\n'.join(lines), encoding='utf-8', newline='\n')
        print(out, len(T))


if __name__ == '__main__':
    main()
