#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use andotp_gui::AndOtpApp;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("andOTP")
            .with_app_id("org.shadowice.andotp")
            .with_inner_size([440.0, 680.0])
            .with_min_inner_size([360.0, 420.0]),
        // OpenGL works on the widest range of machines, including VMs and older GPUs.
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    // Same override as the command-line client.
    let vault = std::env::var_os("ANDOTP_VAULT").map(std::path::PathBuf::from);
    eframe::run_native(
        "andOTP",
        options,
        Box::new(move |cc| {
            let mut app = AndOtpApp::new(cc);
            if let Some(path) = vault {
                app.set_vault_path(path);
            }
            Ok(Box::new(app))
        }),
    )
}
