# TwAura — неофициальный клиент Twitch

## Обзор проекта

TwAura (`tw_aura`) — это неофициальный клиент для Twitch, написанный на Rust. Приложение воспроизводит live-видеопотоки стримеров через HLS, отображает GUI с помощью фреймворка egui (на десктопе — через `eframe`, на Авроре — через `aurora_egui`) и ориентировано в первую очередь на мобильную ОС Аврора (Aurora OS, форк Sailfish OS), а также на десктопный Linux с Wayland.

Основные возможности:
- Просмотр live-стримов Twitch через HLS-плейлисты.
- Декодирование видео и аудио с помощью FFmpeg (`ffmpeg-next`).
- Выбор качества видео из вариантов мастер-плейлиста.
- Загрузка списка подписок через Twitch Helix API.
- Поиск каналов через Twitch Helix API.
- Воспроизведение аудио через PulseAudio (реальный аудиопоток стрима, раньше использовался тестовый WAV).
- Поддержка портретной и альбомной ориентации на Aurora OS.

## Технологический стек

- **Язык:** Rust (edition 2024).
- **GUI:** `egui` + `eframe` (десктоп) / `aurora_egui` (Aurora OS), рендерер Glow, поддержка Wayland.
- **Интеграция с ОС Аврора:** `aurora_services` (wakelock, открытие URI).
- **Асинхронность:** `tokio` (multi-thread runtime).
- **HTTP:** `reqwest` (TLS через `rustls`).
- **Видео/аудио:** `ffmpeg-next` (связка с системным FFmpeg).
- **Аудиовывод:** `libpulse-binding` / `libpulse-simple-binding`.
- **Парсинг HLS:** `m3u8-rs`.
- **Сериализация:** `serde`, `serde_json`.
- **Обработка изображений:** `egui_extras` (загрузка превью через HTTP).
- **Каналы между потоками:** `std::sync::mpsc` (SyncSender/Receiver) и `ring-channel`.

## Структура проекта

```
src/
├── main.rs              # Точка входа: инициализация eframe/aurora_egui
├── app.rs               # Главное окно приложения (MyApp), UI-логика, вкладки
├── config.rs            # Локальная конфигурация (access_token, last_quality)
└── twitch/
    ├── mod.rs           # Объявление подмодулей
    ├── twitch_legacy.rs # Кастомный клиент к Twitch GQL API: playback access token и HLS-плейлист
    ├── stream/          # Движок воспроизведения потока
    │   ├── mod.rs       # YuvFrame
    │   ├── player.rs    # StreamPlayer — управление плеером, чтение HLS, запуск потоков
    │   ├── video.rs     # VideoStream — декодирование видео (YUV420P) и paced-вывод кадров
    │   ├── video_decoder.rs # Тонкая обёртка над FFmpeg video decoder
    │   ├── audio.rs     # AudioStream — вывод аудио через PulseAudio
    │   ├── audio_decoder.rs # Декодирование и ресемплинг аудио в S16LE 48kHz stereo
    │   └── utils.rs     # Вспомогательная печать метаданных потока
    └── ui/              # UI-компоненты
        ├── mod.rs
        ├── auth.rs            # Экран ввода access token
        ├── player.rs          # Окно плеера с оверлейными кнопками и выбором качества
        ├── streams_list.rs    # Список подписок и поиск каналов (Helix API)
        └── yuv_renderer.rs    # OpenGL-рендерер YUV420P кадров

twitch_stream_lib/       # Отдельный crate (не используется, не включён в workspace)
└── src/lib.rs

rpm/                     # Файлы для сборки RPM-пакета
├── com.lmaxyz.TwAura.desktop
├── com.lmaxyz.TwAura.spec
├── icons/               # Иконки разных размеров
└── lib/                 # Нативные библиотеки для целевой архитектуры
```

## Сборка

### Локальная сборка (Linux)

Требуются установленные в системе заголовки и библиотеки FFmpeg, PulseAudio, D-Bus и прочие зависимости для линковки.

```bash
cargo build --release
```

### Кросс-компиляция

Проект использует `cross` и Docker для сборки под `aarch64` и `armv7`.

**aarch64:**
```bash
cross build --release --target aarch64-unknown-linux-gnu
```

**armv7:**
```bash
cross build --release --target armv7-unknown-linux-gnueabihf
```

Для кросс-компиляции используются кастомные образы (`Dockerfile`, `arm.Dockerfile`) и `Cross.toml` с перечислением необходимых `*-dev` пакетов FFmpeg и зависимостей.

### Сборка RPM для Aurora OS

Для сборки и подписи RPM необходим Aurora Platform SDK (`PSDK_DIR`).

**aarch64:**
```bash
./aarch64_build.sh
```

**armv7:**
```bash
./arm_build.sh
```

Скрипты выполняют:
1. `cross build --release` для целевой архитектуры.
2. Копируют бинарник и запускают `mb2` внутри chroot Aurora SDK.
3. Подписывают RPM внешним сертификатом (`rpmsign-external`).
4. Проверяют пакет через `rpm-validator`.

## Особенности исходного кода

- **Язык комментариев:** значительная часть комментариев, особенно объясняющих логику синхронизации и работу буферов, написана на русском языке.
- **Форк зависимостей:** в `Cargo.toml` присутствуют патчи для `glutin` и `egui`/`eframe` из кастомных репозиториев (`lmaxyz/glutin`, `lmaxyz/egui`), необходимые для совместимости с Aurora OS. Патч для `winit` закомментирован.
- **Workspace:** в `Cargo.toml` отсутствует секция `[workspace]`; каталог `twitch_stream_lib` не участвует в сборке основного приложения.
- **Жёстко закодированные значения:**
  - Twitch `Client-ID` (`kimne78kx3ncx6brgo4mv6wki5h1ko`) и `sha256Hash` persisted query для GQL API — в `src/twitch/twitch_legacy.rs`.
  - URL страницы авторизации (`https://twitchaddon.page.link/1Sk5`) — в `src/twitch/ui/auth.rs`.
  - User-Agent для запросов к GQL API — в `src/twitch/twitch_legacy.rs`.
  - **OAuth access token пользователя больше не зашит в коде:** он вводится на экране авторизации и сохраняется в `~/.local/share/com.lmaxyz/TwAura/tw_aura.conf`.
- **Аудио:** аудио-поток декодируется через FFmpeg, ресемплируется в S16LE 48kHz stereo и воспроизводится через PulseAudio. Ранее использовался тестовый WAV-файл, теперь воспроизводится реальный аудиопоток стрима.
- **Видео:** декодер выдаёт сырые кадры в формате YUV420P, которые напрямую передаются в OpenGL-рендерер (`yuv_renderer.rs`). Программное масштабирование до RGB24 и конвертация цветов в CPU отсутствуют.
- **Потоки воспроизведения:**
  1. **Stream reader thread** (`player.rs`) — открывает HLS-ссылку через FFmpeg `input_with_interrupt`, читает пакеты и распределяет их по видео- и аудиоканалам. При обрыве/ошибке потока автоматически пытается переподключиться.
  2. **Video decode thread** (`video.rs`) — получает видеопакеты из `ring_channel`, декодирует их в YUV420P и отправляет готовые кадры в ring buffer.
  3. **Video output thread** (`video.rs`) — извлекает кадры из ring buffer, выдерживает интервал `1/fps` и вызывает callback для рендеринга.
  4. **Audio decode thread** (`player.rs`) — декодирует аудиопакеты и ресемплирует их в S16LE 48kHz stereo.
  5. **Audio feeder thread** (`audio.rs`) — перекачивает PCM-чанки из channel в PulseAudio.
  6. **Display wakelock thread** (только `feature = "aurora"`, `player.rs`) — предотвращает гашение экрана во время воспроизведения.
- **Каналы:**
  - Reader → VideoStream: `ring_channel` с ёмкостью `max(fps * 10, 120)` пакетов. При заполнении старые пакеты перезаписываются.
  - Reader → Audio decoder: `sync_channel(120)` пакетов.
  - Video decoder → Video output: `ring_channel(fps * 7)` (около 7 секунд кадров), с перезаписью старых кадров.
  - Audio decoder → Audio output: `sync_channel(20)` PCM-чанков.
- **Синхронизация:** в текущей версии активная A/V-синхронизация и GOP-skip отсутствуют. Видео выводится с пейсингом `1/fps`, аудио воспроизводится независимо через PulseAudio. В коде остались заглушки и метрики для будущей синхронизации (`last_audio_pts`, `audio_clock`).

## Тестирование

В проекте **отсутствуют** unit-тесты и интеграционные тесты. Проверка функциональности осуществляется вручную через запуск приложения и воспроизведение потока.

## Развертывание

- Бинарник устанавливается в `/usr/bin/com.lmaxyz.TwAura`.
- `.desktop`-файл регистрирует приложение в системе Aurora/Sailfish.
- Иконки копируются в `/usr/share/icons/hicolor/`.
- Нативные библиотеки (если требуются) кладутся в `/usr/share/com.lmaxyz.TwAura/lib/`; бинарник линкуется с `rpath` на эту директорию (см. `.cargo/config.toml`).

## Безопасность

- Пользовательский OAuth access token **не зашит** в исходный код. Он вводится в UI и сохраняется в локальном конфиге (`~/.local/share/com.lmaxyz/TwAura/tw_aura.conf`). Перед распространением стоит подумать о защите этого файла.
- В коде остаются жёстко закодированные публичные константы Twitch: `Client-ID` и `sha256Hash` persisted query в `twitch_legacy.rs`. При изменении со стороны Twitch API их потребуется обновить.
- Приложение запрашивает разрешения `Internet;UserDirs;Audio` (см. `.desktop`).

## Особенности запуска в Aurora OS

- При запуске приложения в песочнице (sandbox) Aurora OS для приложения становится доступна переменная окружения `XDG_RUNTIME_DIR`.
- Внутри этой директории располагаются служебные директории для взаимодействия с сервисами ОС. В частности, там присутствует директория `pulse`, содержащая файлы `dbus-socket`, `native` и `pid`.
- Эти файлы могут использоваться для подключения к PulseAudio внутри sandbox (например, через `PULSE_SERVER` или напрямую через `native`-сокет), что актуально для воспроизведения аудио в приложении.

### Подключение к PulseAudio в Aurora OS

- Код использует `libpulse-simple-binding` и сначала пытается подключиться к серверу по умолчанию.
- Если стандартное подключение не сработало, реализован **fallback**: приложение пытается подключиться напрямую к `$XDG_RUNTIME_DIR/pulse/native`.
- Cookie-файл PulseAudio (`~/.config/pulse/cookie`) в sandbox может отсутствовать; подключение обычно работает и без него (анонимная аутентификация).
- Приложение запрашивает разрешение `Audio` в `.desktop`-файле; без него доступ к PulseAudio в sandbox будет запрещён.

## Полезные команды

```bash
# Локальная отладочная сборка
cargo build

# Локальная релизная сборка (со strip)
cargo build --release

# Кросс-сборка под ARM64
cross build --release --target aarch64-unknown-linux-gnu

# Кросс-сборка под ARMv7
cross build --release --target armv7-unknown-linux-gnueabihf

# Генерация RPM через cargo-generate-rpm (альтернативный вариант)
cargo generate-rpm -a aarch64 --target aarch64-unknown-linux-gnu
```
