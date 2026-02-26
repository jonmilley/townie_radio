use rust_embed::RustEmbed;
use serde::Deserialize;
use crossterm::{
    event::{poll, read, Event, KeyCode},
    execute,
    terminal::{enable_raw_mode, disable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::{
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use rand::{thread_rng, Rng};
use image::GenericImageView;

// ----------------- Embedded Assets -----------------
#[derive(RustEmbed)]
#[folder = "logos/"]
struct Logos;

const STATIONS_JSON: &str = include_str!("../stations.json");

// ----------------- Stations -----------------
#[derive(Deserialize)]
struct Station {
    name: String,
    url: String,
    logo_path: String,
}

fn load_stations() -> Vec<Station> {
    serde_json::from_str(STATIONS_JSON).expect("invalid stations.json")
}

// ----------------- App State -----------------
struct AppState {
    current_station: Option<usize>,
    status: String,
    spectrum: Vec<u8>,
    spinner_index: usize,
    player: Option<Child>,
    loading_since: Option<Instant>,
}

impl AppState {
    fn new() -> Self {
        Self {
            current_station: None,
            status: "OFFLINE".to_string(),
            spectrum: vec![0; 20],
            spinner_index: 0,
            player: None,
            loading_since: None,
        }
    }
}

// ----------------- Audio -----------------
fn play(app: &mut AppState, stations: &[Station]) {
    stop(app);

    if let Some(idx) = app.current_station {
        let station = &stations[idx];

        app.status = "LOADING".to_string();
        app.loading_since = Some(Instant::now());

        match Command::new("ffplay")
            .arg("-nodisp")
            .arg("-loglevel")
            .arg("quiet")
            .arg(&station.url)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                app.player = Some(child);
            }
            Err(_) => {
                app.status = "FAILED".to_string();
                app.loading_since = None;
            }
        }
    }
}

fn stop(app: &mut AppState) {
    if let Some(ref mut p) = app.player {
        let _ = p.kill();
    }
    app.player = None;
    app.status = "OFFLINE".to_string();
}

// ----------------- Spectrum -----------------
fn update_spectrum(app: &mut AppState) {
    if let Some(start) = app.loading_since {
        if start.elapsed().as_secs_f32() > 1.5 {
            app.status = "ONLINE".to_string();
            app.loading_since = None;
        }
    }

    let mut rng = thread_rng();
    for v in &mut app.spectrum {
        if app.status == "ONLINE" {
            let delta: i8 = rng.gen_range(-2..=3);
            *v = ((*v as i8 + delta).clamp(0, 8)) as u8;
        } else {
            *v = v.saturating_sub(1);
        }
    }
    app.spinner_index = (app.spinner_index + 1) % 4;
}

// ----------------- UI -----------------
fn draw_ui(
    app: &AppState,
    stations: &[Station],
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<(), std::io::Error> {
    terminal.draw(|f| {
        let size = f.size();

        // Vertical layout
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints(
                [
                    Constraint::Length(3),      // Title
                    Constraint::Length(3),      // Status
                    Constraint::Percentage(30), // Spectrum
                    Constraint::Min(6),         // Stations + Logo
                ]
                .as_ref(),
            )
            .split(size);

        // Title
        let title_text = match app.current_station {
            Some(idx) => format!("{} - Townie Radio", stations[idx].name),
            None => "Townie Radio".to_string(),
        };
        let title = Paragraph::new(title_text)
            .block(Block::default().borders(Borders::ALL).style(Style::default().fg(Color::Yellow)));
        f.render_widget(title, chunks[0]);

        // Status
        let spinner = ["|", "/", "-", "\\"][app.spinner_index];
        let status = Paragraph::new(format!("STATUS: {} {}", app.status, spinner))
            .block(Block::default().borders(Borders::ALL).style(Style::default().fg(Color::Cyan)));
        f.render_widget(status, chunks[1]);

        // Spectrum
        let spectrum_str: String = app
            .spectrum
            .iter()
            .map(|&v| std::iter::repeat("█").take(v as usize).collect::<String>())
            .collect::<Vec<_>>()
            .join(" ");
        let spectrum_widget = Paragraph::new(spectrum_str)
            .block(Block::default().borders(Borders::ALL).title("SPECTRUM").style(Style::default().fg(Color::Green)));
        f.render_widget(spectrum_widget, chunks[2]);

        // Horizontal split for Stations and Logo
        let bottom_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
            .split(chunks[3]);

        // Stations list
        let mut station_lines = vec![];
        for (i, s) in stations.iter().enumerate() {
            if Some(i) == app.current_station {
                station_lines.push(format!("> [{}] {}", i + 1, s.name));
            } else {
                station_lines.push(format!("  [{}] {}", i + 1, s.name));
            }
        }
        station_lines.push("1-7 = station | SPACE = stop | Q = quit".to_string());

        let stations_paragraph = Paragraph::new(station_lines.join("\n"))
            .block(Block::default().borders(Borders::ALL).style(Style::default().fg(Color::Magenta)));
        f.render_widget(stations_paragraph, bottom_chunks[0]);

        // Station logo — half-block rendering (▀): each char row = 2 pixel rows
        if let Some(idx) = app.current_station {
            let logo_filename = std::path::Path::new(&stations[idx].logo_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            let embedded = Logos::get(logo_filename);
            let img_result = embedded
                .as_ref()
                .and_then(|f| image::load_from_memory(&f.data).ok());
            if let Some(img) = img_result {
                let logo_area = bottom_chunks[1];
                let inner_w = logo_area.width.saturating_sub(2) as u32;
                let inner_h = logo_area.height.saturating_sub(2) as u32;

                if inner_w > 0 && inner_h > 0 {
                    let small = img.resize(inner_w, inner_h * 2, image::imageops::FilterType::Triangle);
                    let actual_w = small.width();
                    let actual_h = small.height();

                    let all_pixels: Vec<image::Rgba<u8>> = small.pixels().map(|(_, _, p)| p).collect();

                    let pixel_rows: Vec<Spans> = (0..(actual_h + 1) / 2)
                        .map(|row| {
                            let spans: Vec<Span> = (0..actual_w)
                                .map(|col| {
                                    let top = all_pixels[(row * 2 * actual_w + col) as usize].0;
                                    let bot = if row * 2 + 1 < actual_h {
                                        all_pixels[((row * 2 + 1) * actual_w + col) as usize].0
                                    } else {
                                        [0, 0, 0, 255]
                                    };
                                    Span::styled(
                                        "▀",
                                        Style::default()
                                            .fg(Color::Rgb(top[0], top[1], top[2]))
                                            .bg(Color::Rgb(bot[0], bot[1], bot[2])),
                                    )
                                })
                                .collect();
                            Spans::from(spans)
                        })
                        .collect();

                    let logo_widget = Paragraph::new(pixel_rows)
                        .block(Block::default().borders(Borders::ALL).title("LOGO"));
                    f.render_widget(logo_widget, logo_area);
                }
            }
        }
    })?;
    Ok(())
}

// ----------------- Main -----------------
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stations = load_stations();

    let mut stdout = std::io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app = Arc::new(Mutex::new(AppState::new()));

    // Spectrum animation thread
    let app_clone = Arc::clone(&app);
    thread::spawn(move || loop {
        {
            let mut a = app_clone.lock().unwrap();
            update_spectrum(&mut a);
        }
        thread::sleep(Duration::from_millis(80));
    });

    loop {
        {
            let a = app.lock().unwrap();
            draw_ui(&a, &stations, &mut terminal)?;
        }

        if poll(Duration::from_millis(50))? {
            if let Event::Key(key) = read()? {
                let mut a = app.lock().unwrap();

                match key.code {
                    KeyCode::Char('q') => {
                        stop(&mut a);
                        break;
                    }
                    KeyCode::Char(' ') => {
                        stop(&mut a);
                        a.current_station = None;
                    }
                    KeyCode::Char(c) => {
                        if let Some(digit) = c.to_digit(10) {
                            if digit == 0 {
                                stop(&mut a);
                                break;
                            }
                            let idx = (digit - 1) as usize;
                            if idx < stations.len() {
                                stop(&mut a);
                                a.current_station = Some(idx);
                                play(&mut a, &stations);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
