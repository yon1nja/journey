use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use alacritty_terminal::event::EventListener;
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::Processor;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

#[derive(Clone)]
pub struct EventProxy {
    pty_writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: alacritty_terminal::event::Event) {
        if let alacritty_terminal::event::Event::PtyWrite(text) = event {
            if let Ok(mut writer) = self.pty_writer.lock() {
                let _ = writer.write_all(text.as_bytes());
            }
        }
    }
}

struct TermSize {
    rows: usize,
    cols: usize,
}

impl alacritty_terminal::grid::Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct EmbeddedTerminal {
    term: Arc<Mutex<Term<EventProxy>>>,
    pty_writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    alive: Arc<Mutex<bool>>,
}

impl EmbeddedTerminal {
    pub fn spawn(shell: &str, cwd: &Path, rows: u16, cols: u16) -> Result<Self> {
        let pty_system = native_pty_system();
        let pty_size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = pty_system.openpty(pty_size).context("failed to open PTY")?;

        let mut cmd = CommandBuilder::new(shell);
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");

        pair.slave
            .spawn_command(cmd)
            .context("failed to spawn shell")?;
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .context("failed to take PTY writer")?;
        let pty_writer = Arc::new(Mutex::new(writer));

        let event_proxy = EventProxy {
            pty_writer: pty_writer.clone(),
        };

        let config = TermConfig::default();
        let size = TermSize {
            rows: rows as usize,
            cols: cols as usize,
        };
        let term = Term::new(config, &size, event_proxy);
        let term = Arc::new(Mutex::new(term));

        let alive = Arc::new(Mutex::new(true));

        let reader = pair
            .master
            .try_clone_reader()
            .context("failed to clone PTY reader")?;

        Self::start_reader_thread(reader, term.clone(), alive.clone());

        Ok(Self {
            term,
            pty_writer,
            master: pair.master,
            alive,
        })
    }

    fn start_reader_thread(
        mut reader: Box<dyn Read + Send>,
        term: Arc<Mutex<Term<EventProxy>>>,
        alive: Arc<Mutex<bool>>,
    ) {
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut processor: Processor = Processor::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut term) = term.lock() {
                            processor.advance(&mut *term, &buf[..n]);
                        }
                    }
                    Err(_) => break,
                }
            }
            if let Ok(mut alive) = alive.lock() {
                *alive = false;
            }
        });
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("failed to resize PTY")?;

        if let Ok(mut term) = self.term.lock() {
            let size = TermSize {
                rows: rows as usize,
                cols: cols as usize,
            };
            term.resize(size);
        }
        Ok(())
    }

    pub fn with_term<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Term<EventProxy>) -> R,
    {
        let term = self.term.lock().expect("term lock poisoned");
        f(&term)
    }

    pub fn is_alive(&self) -> bool {
        self.alive.lock().map(|alive| *alive).unwrap_or(false)
    }

    pub fn write_key(&self, key: &KeyEvent) {
        let bytes = key_to_bytes(key);
        if !bytes.is_empty() {
            if let Ok(mut writer) = self.pty_writer.lock() {
                let _ = writer.write_all(&bytes);
            }
        }
    }
}

fn key_to_bytes(key: &KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        KeyCode::Char(c) if ctrl => {
            let byte = (c as u8).wrapping_sub(b'a').wrapping_add(1);
            if alt {
                vec![0x1b, byte]
            } else {
                vec![byte]
            }
        }
        KeyCode::Char(c) => {
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            if alt {
                let mut v = vec![0x1b];
                v.extend_from_slice(s.as_bytes());
                v
            } else {
                s.as_bytes().to_vec()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                vec![0x1b, b'[', b'Z']
            } else {
                vec![b'\t']
            }
        }
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => vec![0x1b, b'[', b'A'],
        KeyCode::Down => vec![0x1b, b'[', b'B'],
        KeyCode::Right => vec![0x1b, b'[', b'C'],
        KeyCode::Left => vec![0x1b, b'[', b'D'],
        KeyCode::Home => vec![0x1b, b'[', b'H'],
        KeyCode::End => vec![0x1b, b'[', b'F'],
        KeyCode::PageUp => vec![0x1b, b'[', b'5', b'~'],
        KeyCode::PageDown => vec![0x1b, b'[', b'6', b'~'],
        KeyCode::Insert => vec![0x1b, b'[', b'2', b'~'],
        KeyCode::Delete => vec![0x1b, b'[', b'3', b'~'],
        KeyCode::F(1) => vec![0x1b, b'O', b'P'],
        KeyCode::F(2) => vec![0x1b, b'O', b'Q'],
        KeyCode::F(3) => vec![0x1b, b'O', b'R'],
        KeyCode::F(4) => vec![0x1b, b'O', b'S'],
        KeyCode::F(n @ 5..=12) => {
            let codes = [b"15", b"17", b"18", b"19", b"20", b"21", b"23", b"24"];
            let idx = (n - 5) as usize;
            if idx < codes.len() {
                let mut v = vec![0x1b, b'['];
                v.extend_from_slice(codes[idx]);
                v.push(b'~');
                v
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}
