//! QR-матрица из fingerprint remote-auth — рендерим модулями div,
//! без ассетов и вебвью (qrcode-крейт из дерева ядра).

/// Сгенерировать матрицу QR-кода: (модули row-major, ширина стороны).
pub fn matrix(data: &str) -> anyhow::Result<(Vec<bool>, usize)> {
    let code = qrcode::QrCode::new(data.as_bytes())?;
    let width = code.width();
    let modules = code
        .to_colors()
        .into_iter()
        .map(|c| matches!(c, qrcode::Color::Dark))
        .collect();
    Ok((modules, width))
}
