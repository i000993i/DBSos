# DBSos GUI Architecture

## Design Philosophy
- **Visual**: macOS glass (translucent panels, vibrancy, blur, rounded corners)
- **UX**: Windows 10 convenience (taskbar, start menu, snap, familiar shortcuts)
- **Stack**: Linux-style modularity (compositor → WM → shell → apps), all Rust no_std

## Layer Stack (bottom → top)

```
┌───────────────────────────────────────────────────┐
│  5. Applications                                  │
│     Terminal, FileManager, Settings, TextEditor   │
├───────────────────────────────────────────────────┤
│  4. Desktop Shell                                 │
│     Taskbar, StartMenu, SystemTray, Notifications │
├───────────────────────────────────────────────────┤
│  3. Window Manager                                │
│     Z-order, Snap, Resize, Drag, Decorations      │
├───────────────────────────────────────────────────┤
│  2. Compositor                                    │
│     Dirty-rect, Layer tree, Glass blur, VSync     │
├───────────────────────────────────────────────────┤
│  1. Input + Display backends                      │
│     Mouse/Keyboard IRQ, GOP framebuffer, Cursor   │
└───────────────────────────────────────────────────┘
```

## Components

### 1. Input Backend (`input.rs`)
- PS/2 mouse: absolute position, relative deltas
- PS/2 keyboard: scancode → keycode mapping
- Event queue: InputEvent enum (MouseMove, MouseButton, KeyPress, KeyRelease)
- Debounce, repeat rate

### 2. Compositor (`compositor.rs`)
- **Layer tree**: Desktop → Wallpaper → Windows (back→front) → Shell → Cursor
- **Dirty rectangles**: only repaint changed regions
- **Glass effect**: translucency = alpha blend with background
  - Blur: 3x3 or 5x5 box blur of framebuffer region behind window
  - Alpha: per-pixel alpha for glass panels
- **Double buffer**: render to back buffer, swap on vsync
- **Damage tracking**: per-window damage rects, merge overlapping

### 3. Window Manager (`wm.rs`)
- **Window states**: Normal, Maximized, Minimized, SnappedLeft, SnappedRight
- **Z-order**: Active window on top, focus follows click
- **Decorations**:
  - Title bar: macOS style (traffic lights: close/minimize/maximize)
  - Rounded corners: 10px radius
  - Drop shadow: gaussian blur beneath window
- **Snap**: drag to left/right edge → fill half screen (Win10)
- **Focus**: click to focus, Alt+Tab cycle
- **Desktop**: icon grid on wallpaper

### 4. Desktop Shell (`shell.rs`)
- **Taskbar** (bottom, 48px):
  - Left: Start button (DB logo)
  - Center: running app icons (pinned + active)
  - Right: system tray (clock, volume, network)
  - Glass: translucent blur behind taskbar
- **Start Menu**:
  - Left: pinned apps grid
  - Right: app list (alphabetical)
  - Bottom: power, settings, user
  - Glass background with blur
- **Notifications**: slide-in from top-right, auto-dismiss

### 5. Visual Style

#### Colors (Glass Dark)
```
Background:      #1E1E2E (deep navy)
Surface:         #2D2D3F (card/panel)
Glass BG:        #2D2D3F @ 80% alpha + blur
Accent:          #7C3AED (purple, like macOS Monterey)
Text Primary:    #E2E2F0
Text Secondary:  #8888AA
Border:          #3D3D5540
Shadow:          #00000060
Taskbar BG:      #1A1A2C @ 85% alpha + blur
Title Bar:       #252540 @ 90% alpha + blur
Hover:           #FFFFFF10
Active:          #7C3AED30
Close button:    #FF5F57
Minimize button: #FFBD2E
Maximize button: #28C840
```

#### Typography
- Font: 8x8 bitmap (existing) for now, upgradeable to TTF later
- Title: 8px, bold (simulated via double-draw)
- Body: 8px regular
- Spacing: 4px grid

#### Widget Library (`widget.rs`)
- Button: hover/press states, rounded corners
- TextBox: cursor, selection, scroll
- Label: multiline support
- Scrollbar: thin, auto-hide
- Panel: glass container
- Tab bar
- Dropdown/ComboBox

## Data Flow

```
IRQ → InputEvent → [WM] → Window State → [Compositor] → Framebuffer
                                    ↑                          ↓
                              App Message ←──── App Event ←────┘
```

1. Mouse IRQ → update position, button state
2. Input backend → generates InputEvent
3. WM processes event:
   - Click on window → focus, raise, pass to app
   - Click on taskbar → start menu / app switch
   - Drag title bar → move window
   - Drag edge → resize
   - Key press → focused app receives
4. Compositor:
   - Collects dirty rects
   - Renders layer tree (back→front)
   - Applies glass blur where needed
   - Draws cursor last
5. Display backend → writes to GOP framebuffer

## File Structure

```
kernel/src/
├── gui/
│   ├── mod.rs              # GUI entry, main loop
│   ├── compositor.rs       # Layer tree, dirty-rect, rendering
│   ├── wm.rs               # Window manager
│   ├── shell.rs            # Taskbar, start menu, tray
│   ├── input.rs            # Input events, mouse, keyboard
│   ├── widget.rs           # UI widgets (Button, TextBox, etc.)
│   ├── theme.rs            # Colors, dimensions, style constants
│   ├── render.rs           # Low-level drawing (rect, blur, text)
│   └── apps/
│       ├── mod.rs
│       ├── terminal.rs     # Terminal emulator
│       ├── filemanager.rs  # File manager
│       ├── settings.rs     # System settings
│       └── texteditor.rs   # Text editor
```
