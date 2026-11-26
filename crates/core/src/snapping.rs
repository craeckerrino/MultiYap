//! Einrasten einer Trennlinie (bisher app/layout/snapping.ts): Mindestgröße einhalten, dann an ½, ⅓, ⅔ oder an Linien
//! benachbarter Bereiche einrasten (außer bei gehaltener Alt-Taste).

/// Feste Einrastpunkte.
pub const SNAP_RATIOS: [f64; 3] = [1.0 / 3.0, 1.0 / 2.0, 2.0 / 3.0];
/// Abstand in Pixeln, ab dem eine Linie einrastet.
pub const SNAP_DISTANCE_PX: f64 = 10.0;
/// Mindestgröße eines Bereichs in Pixeln.
pub const MIN_AREA_PX: f64 = 160.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Snapped {
    /// An einem festen Anteil (½, ⅓, ⅔).
    Ratio(f64),
    /// An der Linie eines Nachbarbereichs.
    Neighbour,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapResult {
    /// Position in Pixeln ab dem Anfang der Teilung.
    pub position: f64,
    pub snapped_to: Option<Snapped>,
}

pub fn snap_position(raw: f64, size: f64, neighbour_lines: &[f64], snapping: bool) -> SnapResult {
    let min = MIN_AREA_PX.min(size / 2.0);
    let clamped = raw.max(min).min(size - min);
    let mut best = SnapResult { position: clamped, snapped_to: None };
    if !snapping {
        return best;
    }
    let mut best_distance = SNAP_DISTANCE_PX + 1.0;
    let candidates = SNAP_RATIOS.iter().map(|r| (r * size, Snapped::Ratio(*r))).chain(neighbour_lines.iter().map(|l| (*l, Snapped::Neighbour)));
    for (position, kind) in candidates {
        let distance = (position - clamped).abs();
        if distance <= SNAP_DISTANCE_PX && distance < best_distance && position >= min && position <= size - min {
            best = SnapResult { position, snapped_to: Some(kind) };
            best_distance = distance;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn einrasten() {
        assert_eq!(snap_position(595.0, 1200.0, &[], true), SnapResult { position: 600.0, snapped_to: Some(Snapped::Ratio(0.5)) });
        assert_eq!(snap_position(405.0, 1200.0, &[], true).snapped_to, Some(Snapped::Ratio(1.0 / 3.0)));
        assert_eq!(snap_position(792.0, 1200.0, &[], true).snapped_to, Some(Snapped::Ratio(2.0 / 3.0)));
        // Linien benachbarter Bereiche.
        assert_eq!(snap_position(503.0, 1200.0, &[500.0], true), SnapResult { position: 500.0, snapped_to: Some(Snapped::Neighbour) });
        // Alt setzt das Einrasten aus.
        assert_eq!(snap_position(595.0, 1200.0, &[], false), SnapResult { position: 595.0, snapped_to: None });
        // Mindestgröße.
        assert_eq!(snap_position(10.0, 1200.0, &[], false).position, MIN_AREA_PX);
        assert_eq!(snap_position(1190.0, 1200.0, &[], false).position, 1200.0 - MIN_AREA_PX);
        // Weit weg von allen Punkten bleibt die Position frei.
        assert_eq!(snap_position(700.0, 1200.0, &[], true), SnapResult { position: 700.0, snapped_to: None });
    }
}
