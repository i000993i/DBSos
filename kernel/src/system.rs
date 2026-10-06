//! System Levels — архитектура DBSos
//!
//! LEVEL_0_SYSTEM — ядро (memory, VM, heap, interrupts, drivers, scheduler, VFS)
//! LEVEL_1_SYSTEM — TUI (shell, Ubuntu Server-like: ввод, запуск, скачивание)
//! LEVEL_2_SYSTEM — GUI (Plasma, Ubuntu Desktop-like: рабочий стол, программы, игры)

use crate::driver::uart;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    L0Kernel = 0,
    L1Tui = 1,
    L2Plasma = 2,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::L0Kernel => "LEVEL_0_SYSTEM (kernel)",
            Level::L1Tui => "LEVEL_1_SYSTEM (TUI)",
            Level::L2Plasma => "LEVEL_2_SYSTEM (Plasma)",
        }
    }
    pub fn desc(self) -> &'static str {
        match self {
            Level::L0Kernel => "kernel: memory, drivers, scheduler, VFS",
            Level::L1Tui => "TUI: shell, pkg, wget, Ubuntu Server-like",
            Level::L2Plasma => "GUI: Plasma, desktop, apps, Ubuntu Desktop-like",
        }
    }
}

static mut CURRENT: Level = Level::L0Kernel;

pub fn current() -> Level { unsafe { CURRENT } }

pub fn set(level: Level) {
    unsafe { CURRENT = level; }
    uart::write_str("[SYS] level -> ");
    uart::write_str(level.name());
    uart::write_str("\r\n");
}

pub fn boot_banner() {
    uart::write_str("\r\n=== DBSos System Levels ===\r\n");
    for lvl in [Level::L0Kernel, Level::L1Tui, Level::L2Plasma] {
        uart::write_str(if lvl as u8 == unsafe { CURRENT } as u8 { " * " } else { "   " });
        uart::write_str(lvl.name());
        uart::write_str(" — ");
        uart::write_str(lvl.desc());
        uart::write_str("\r\n");
    }
}
