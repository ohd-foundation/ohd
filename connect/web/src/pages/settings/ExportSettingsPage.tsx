/**
 * Settings → Export.
 *
 * Storage's `Export.Export` and `Export.GenerateDoctorPdf` are stubbed
 * (return Unimplemented). This page is a placeholder that:
 *   - Explains what export will do once it ships.
 *   - Renders a disabled "Export everything" button.
 *   - Renders a disabled "Doctor PDF" button.
 *   - Documents the migration path (`MigrateInit`/`MigrateFinalize`) for users
 *     switching deployment modes.
 */
export function ExportSettingsPage() {
  return (
    <div data-testid="settings-export">
      <div className="banner warn">
        Export and import are stubbed in the current OHD Storage build (
        <code>storage/STATUS.md</code> "ReadSamples / Export / Import: v1.x").
        The buttons below activate once the storage RPCs land.
      </div>

      <div className="card">
        <div className="card-title">
          <h3>Full lossless export</h3>
        </div>
        <p className="muted" style={{ marginTop: 0 }}>
          Stream every event, every channel value, every grant rule, every audit-log row,
          every attachment to a single signed file. Encrypts to a passphrase you supply.
          Restorable to any OHD Storage instance via <code>Import</code>.
        </p>
        <button className="btn" disabled>
          Export everything (TBD)
        </button>
      </div>

      <div className="card">
        <div className="card-title">
          <h3>Doctor PDF</h3>
        </div>
        <p className="muted" style={{ marginTop: 0 }}>
          A curated PDF for in-person sharing — recent vitals, active medications, allergies,
          relevant timeline. Generated server-side with a deterministic layout so two
          generations of the same window are byte-identical.
        </p>
        <button className="btn" disabled>
          Generate doctor PDF (TBD)
        </button>
      </div>

      <div className="card">
        <div className="card-title">
          <h3>Migration assistant</h3>
        </div>
        <p className="muted" style={{ marginTop: 0 }}>
          Moving between deployment modes (on-device → cloud, cloud → self-hosted) is the
          two-step flow <code>Export.MigrateInit</code> → <code>Export.MigrateFinalize</code>.
          The new instance verifies the source-instance signature on the export, accepts the
          stream, and the source instance flips read-only for a finite cutover window. v0.x.
        </p>
        <button className="btn" disabled>
          Migrate to a new instance (TBD)
        </button>
      </div>
    </div>
  );
}
