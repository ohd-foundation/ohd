import { useState, type FormEvent } from "react";
import { Modal } from "../components/Modal";
import { useToast } from "../components/Toast";
import {
  submitBloodPressure,
  submitGlucose,
  submitGlucoseMgDl,
  submitHeartRate,
  submitMeal,
  submitMedication,
  submitMood,
  submitNote,
  submitSymptom,
  submitTemperatureC,
  submitTemperatureF,
} from "../ohdc/store";

/**
 * The Log page — quick-entry tile grid. Each tile opens a typed modal that
 * collects the right fields, calls the matching `submit*` helper, and
 * surfaces the outcome via toast. Per the v0.1 surface, eight tiles cover
 * the std.* registry the storage server seeds (`002_std_registry.sql`)
 * plus `std.clinical_note`.
 */

type LogKind =
  | "glucose"
  | "heart_rate"
  | "temperature"
  | "blood_pressure"
  | "medication"
  | "symptom"
  | "meal"
  | "mood"
  | "note";

const TILES: Array<{ kind: LogKind; label: string; sub: string; icon: string }> = [
  { kind: "glucose", label: "Glucose", sub: "mmol/L or mg/dL", icon: "◐" },
  { kind: "heart_rate", label: "Heart rate", sub: "Resting bpm", icon: "♡" },
  { kind: "blood_pressure", label: "Blood pressure", sub: "Systolic / diastolic", icon: "◇" },
  { kind: "temperature", label: "Temperature", sub: "°C or °F", icon: "◊" },
  { kind: "medication", label: "Medication", sub: "Dose & status", icon: "℞" },
  { kind: "symptom", label: "Symptom", sub: "Severity 1–10", icon: "△" },
  { kind: "meal", label: "Meal", sub: "Description, kcal", icon: "□" },
  { kind: "mood", label: "Mood", sub: "Mood + energy", icon: "◯" },
];

export function LogPage() {
  const [open, setOpen] = useState<LogKind | null>(null);
  return (
    <section data-testid="log-page">
      <header className="page-header">
        <div>
          <h1>Log</h1>
          <p>Quickly record a measurement or event. Goes straight into your OHD storage.</p>
        </div>
      </header>

      <div className="tile-grid">
        {TILES.map((t) => (
          <button key={t.kind} className="tile" type="button" onClick={() => setOpen(t.kind)} data-testid={`tile-${t.kind}`}>
            <span className="tile-icon" aria-hidden="true">
              {t.icon}
            </span>
            <span className="tile-label">{t.label}</span>
            <span className="tile-sub">{t.sub}</span>
          </button>
        ))}
        <button className="tile" type="button" onClick={() => setOpen("note")} data-testid="tile-note">
          <span className="tile-icon" aria-hidden="true">
            ✎
          </span>
          <span className="tile-label">Free-text note</span>
          <span className="tile-sub">Goes to clinical notes</span>
        </button>
      </div>

      <LogModal kind={open} onClose={() => setOpen(null)} />
    </section>
  );
}

function LogModal({ kind, onClose }: { kind: LogKind | null; onClose: () => void }) {
  const toast = useToast();
  if (!kind) return null;
  const titles: Record<LogKind, string> = {
    glucose: "Log glucose",
    heart_rate: "Log heart rate",
    blood_pressure: "Log blood pressure",
    temperature: "Log temperature",
    medication: "Log medication",
    symptom: "Log symptom",
    meal: "Log meal",
    mood: "Log mood",
    note: "Add note",
  };
  const handle = (fn: () => Promise<void>, ok: string) => async (e?: FormEvent) => {
    e?.preventDefault();
    try {
      await fn();
      toast.show(ok, "success");
      onClose();
    } catch (err) {
      toast.show(`Submit failed: ${(err as Error).message ?? String(err)}`, "error");
    }
  };
  return (
    <Modal open onClose={onClose} title={titles[kind]} subtitle="Stored under your self-session.">
      {kind === "glucose" ? <GlucoseForm onSubmit={handle} /> : null}
      {kind === "heart_rate" ? <HeartRateForm onSubmit={handle} /> : null}
      {kind === "blood_pressure" ? <BPForm onSubmit={handle} /> : null}
      {kind === "temperature" ? <TempForm onSubmit={handle} /> : null}
      {kind === "medication" ? <MedicationForm onSubmit={handle} /> : null}
      {kind === "symptom" ? <SymptomForm onSubmit={handle} /> : null}
      {kind === "meal" ? <MealForm onSubmit={handle} /> : null}
      {kind === "mood" ? <MoodForm onSubmit={handle} /> : null}
      {kind === "note" ? <NoteForm onSubmit={handle} /> : null}
    </Modal>
  );
}

interface FormProps {
  onSubmit: (
    fn: () => Promise<void>,
    ok: string,
  ) => (e?: FormEvent) => Promise<void>;
}

function GlucoseForm({ onSubmit }: FormProps) {
  const [value, setValue] = useState("");
  const [unit, setUnit] = useState<"mmol/L" | "mg/dL">("mmol/L");
  return (
    <form
      onSubmit={onSubmit(async () => {
        const n = parseFloat(value);
        if (!Number.isFinite(n)) throw new Error("invalid value");
        if (unit === "mmol/L") await submitGlucose(n);
        else await submitGlucoseMgDl(n);
      }, "Glucose recorded")}
    >
      <div className="modal-body">
        <div className="field-row cols-2">
          <label className="field">
            Value
            <input type="number" inputMode="decimal" step="any" value={value} onChange={(e) => setValue(e.target.value)} autoFocus required />
          </label>
          <label className="field">
            Unit
            <select value={unit} onChange={(e) => setUnit(e.target.value as "mmol/L" | "mg/dL")}>
              <option value="mmol/L">mmol/L</option>
              <option value="mg/dL">mg/dL</option>
            </select>
          </label>
        </div>
      </div>
      <ModalFoot />
    </form>
  );
}

function HeartRateForm({ onSubmit }: FormProps) {
  const [bpm, setBpm] = useState("");
  return (
    <form
      onSubmit={onSubmit(async () => {
        const n = parseFloat(bpm);
        if (!Number.isFinite(n)) throw new Error("invalid bpm");
        await submitHeartRate(n);
      }, "Heart rate recorded")}
    >
      <div className="modal-body">
        <label className="field">
          Bpm
          <input type="number" inputMode="numeric" value={bpm} onChange={(e) => setBpm(e.target.value)} autoFocus required />
        </label>
      </div>
      <ModalFoot />
    </form>
  );
}

function BPForm({ onSubmit }: FormProps) {
  const [sys, setSys] = useState("");
  const [dia, setDia] = useState("");
  return (
    <form
      onSubmit={onSubmit(async () => {
        const s = parseFloat(sys);
        const d = parseFloat(dia);
        if (!Number.isFinite(s) || !Number.isFinite(d)) throw new Error("invalid bp");
        await submitBloodPressure(s, d);
      }, "Blood pressure recorded")}
    >
      <div className="modal-body">
        <div className="field-row cols-2">
          <label className="field">
            Systolic
            <input type="number" inputMode="numeric" value={sys} onChange={(e) => setSys(e.target.value)} autoFocus required />
          </label>
          <label className="field">
            Diastolic
            <input type="number" inputMode="numeric" value={dia} onChange={(e) => setDia(e.target.value)} required />
          </label>
        </div>
      </div>
      <ModalFoot />
    </form>
  );
}

function TempForm({ onSubmit }: FormProps) {
  const [v, setV] = useState("");
  const [unit, setUnit] = useState<"C" | "F">("C");
  return (
    <form
      onSubmit={onSubmit(async () => {
        const n = parseFloat(v);
        if (!Number.isFinite(n)) throw new Error("invalid temp");
        if (unit === "C") await submitTemperatureC(n);
        else await submitTemperatureF(n);
      }, "Temperature recorded")}
    >
      <div className="modal-body">
        <div className="field-row cols-2">
          <label className="field">
            Value
            <input type="number" inputMode="decimal" step="any" value={v} onChange={(e) => setV(e.target.value)} autoFocus required />
          </label>
          <label className="field">
            Unit
            <select value={unit} onChange={(e) => setUnit(e.target.value as "C" | "F")}>
              <option value="C">°C</option>
              <option value="F">°F</option>
            </select>
          </label>
        </div>
      </div>
      <ModalFoot />
    </form>
  );
}

function MedicationForm({ onSubmit }: FormProps) {
  const [name, setName] = useState("");
  const [dose, setDose] = useState("");
  const [status, setStatus] = useState<"taken" | "skipped" | "late" | "refused">("taken");
  return (
    <form
      onSubmit={onSubmit(async () => {
        if (!name.trim()) throw new Error("invalid name");
        const d = dose ? parseFloat(dose) : null;
        await submitMedication(name.trim(), d != null && Number.isFinite(d) ? d : null, status);
      }, "Medication recorded")}
    >
      <div className="modal-body">
        <label className="field">
          Name
          <input type="text" value={name} onChange={(e) => setName(e.target.value)} autoFocus required />
        </label>
        <div className="field-row cols-2">
          <label className="field">
            Dose (mg)
            <input type="number" inputMode="decimal" step="any" value={dose} onChange={(e) => setDose(e.target.value)} />
          </label>
          <label className="field">
            Status
            <select value={status} onChange={(e) => setStatus(e.target.value as "taken" | "skipped" | "late" | "refused")}>
              <option value="taken">Taken</option>
              <option value="skipped">Skipped</option>
              <option value="late">Late</option>
              <option value="refused">Refused</option>
            </select>
          </label>
        </div>
      </div>
      <ModalFoot />
    </form>
  );
}

function SymptomForm({ onSubmit }: FormProps) {
  const [name, setName] = useState("");
  const [severity, setSeverity] = useState(5);
  return (
    <form
      onSubmit={onSubmit(async () => {
        if (!name.trim()) throw new Error("invalid name");
        await submitSymptom(name.trim(), severity);
      }, "Symptom logged")}
    >
      <div className="modal-body">
        <label className="field">
          Symptom
          <input type="text" value={name} onChange={(e) => setName(e.target.value)} autoFocus required />
        </label>
        <label className="field">
          Severity ({severity}/10)
          <input type="range" min={1} max={10} step={1} value={severity} onChange={(e) => setSeverity(Number(e.target.value))} />
        </label>
      </div>
      <ModalFoot />
    </form>
  );
}

function MealForm({ onSubmit }: FormProps) {
  const [desc, setDesc] = useState("");
  const [kcal, setKcal] = useState("");
  return (
    <form
      onSubmit={onSubmit(async () => {
        if (!desc.trim()) throw new Error("invalid description");
        const k = kcal ? parseFloat(kcal) : null;
        await submitMeal(desc.trim(), k != null && Number.isFinite(k) ? k : null);
      }, "Meal logged")}
    >
      <div className="modal-body">
        <label className="field">
          Description
          <textarea value={desc} onChange={(e) => setDesc(e.target.value)} autoFocus required />
        </label>
        <label className="field">
          Approx. kcal (optional)
          <input type="number" inputMode="numeric" value={kcal} onChange={(e) => setKcal(e.target.value)} />
        </label>
      </div>
      <ModalFoot />
    </form>
  );
}

function MoodForm({ onSubmit }: FormProps) {
  const [mood, setMood] = useState("");
  const [energy, setEnergy] = useState(5);
  return (
    <form
      onSubmit={onSubmit(async () => {
        if (!mood.trim()) throw new Error("invalid mood");
        await submitMood(mood.trim(), energy);
      }, "Mood logged")}
    >
      <div className="modal-body">
        <label className="field">
          Mood
          <input type="text" value={mood} onChange={(e) => setMood(e.target.value)} placeholder="e.g. calm, anxious, focused" autoFocus required />
        </label>
        <label className="field">
          Energy ({energy}/10)
          <input type="range" min={1} max={10} step={1} value={energy} onChange={(e) => setEnergy(Number(e.target.value))} />
        </label>
      </div>
      <ModalFoot />
    </form>
  );
}

function NoteForm({ onSubmit }: FormProps) {
  const [text, setText] = useState("");
  return (
    <form
      onSubmit={onSubmit(async () => {
        if (!text.trim()) throw new Error("invalid text");
        await submitNote(text.trim());
      }, "Note saved")}
    >
      <div className="modal-body">
        <label className="field">
          Note
          <textarea value={text} onChange={(e) => setText(e.target.value)} autoFocus required />
        </label>
      </div>
      <ModalFoot />
    </form>
  );
}

function ModalFoot() {
  return (
    <div className="modal-foot">
      <button type="submit" className="btn btn-accent">
        Submit
      </button>
    </div>
  );
}
