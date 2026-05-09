import { useEffect, useState, type FormEvent } from "react";
import type { EventType } from "../types";
import {
  submitFood,
  submitImaging,
  submitLab,
  submitMedication,
  submitNote,
  submitSymptom,
  submitVital,
} from "../mock/store";
import { useToast } from "./Toast";

interface Props {
  open: boolean;
  onClose: () => void;
  patientSlug: string;
  patientLabel: string;
  approvalMode: string;
  /**
   * Which event type the modal is collecting. Drives the form fields and the
   * mock submission target.
   */
  eventType: EventType;
  operatorName: string;
}

/**
 * Single-modal solution that re-skins per event type. Handles the mandatory
 * confirmation step (SPEC §3.3 #3, §6.3 #1) by echoing the active patient's
 * label back to the operator before the actual submit fires.
 */
export function NewEntryModal({
  open,
  onClose,
  patientSlug,
  patientLabel,
  approvalMode,
  eventType,
  operatorName,
}: Props) {
  const [stage, setStage] = useState<"form" | "confirm">("form");
  const [pending, setPending] = useState<null | (() => void)>(null);
  const toast = useToast();

  // Reset internal stage when reopened.
  useEffect(() => {
    if (open) {
      setStage("form");
      setPending(null);
    }
  }, [open, eventType]);

  if (!open) return null;

  const willAutoCommit =
    approvalMode === "never_required" ||
    (approvalMode === "auto_for_event_types" && AUTO_COMMIT_TYPES.includes(eventType));

  const onSubmit = (action: () => void) => {
    setPending(() => action);
    setStage("confirm");
  };

  const onConfirm = () => {
    pending?.();
    toast.show(
      willAutoCommit
        ? `Submitted to ${patientLabel} — auto-committed (per grant policy).`
        : `Submitted to ${patientLabel} — awaiting patient approval.`,
      "success",
    );
    onClose();
  };

  return (
    <div className="modal-overlay" onClick={onClose} role="dialog" aria-modal="true">
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{TITLES[eventType]}</h3>
          <div className="modal-sub">
            For <strong>{patientLabel}</strong> ·{" "}
            <span className="mono">{willAutoCommit ? "auto-commit" : "queues for patient approval"}</span>
          </div>
        </div>

        {stage === "form" && (
          <FormForType
            eventType={eventType}
            patientSlug={patientSlug}
            operatorName={operatorName}
            onCancel={onClose}
            onReady={onSubmit}
          />
        )}

        {stage === "confirm" && (
          <>
            <div className="modal-body">
              <div className="confirm-banner">
                Submitting to <strong>{patientLabel}</strong> — confirm?
              </div>
              <p className="muted" style={{ margin: 0, fontSize: 12 }}>
                Per SPEC §3.3 multi-patient context safety: every clinical write requires explicit
                acknowledgement of the active patient's label.
              </p>
            </div>
            <div className="modal-foot">
              <button className="btn" onClick={() => setStage("form")}>
                Back
              </button>
              <button className="btn btn-accent" onClick={onConfirm}>
                Confirm submit
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

const TITLES: Record<EventType, string> = {
  vital: "New vital",
  medication: "Prescribe medication",
  symptom: "Log symptom",
  food: "Log food",
  lab: "Submit lab result",
  imaging: "Submit imaging finding",
  note: "Add clinical note",
};

const AUTO_COMMIT_TYPES: EventType[] = ["lab", "note"];

interface FormProps {
  eventType: EventType;
  patientSlug: string;
  operatorName: string;
  onCancel: () => void;
  onReady: (action: () => void) => void;
}

function FormForType({ eventType, patientSlug, operatorName, onCancel, onReady }: FormProps) {
  switch (eventType) {
    case "note":
      return <NoteForm onCancel={onCancel} onReady={onReady} patientSlug={patientSlug} author={operatorName} />;
    case "vital":
      return <VitalForm onCancel={onCancel} onReady={onReady} patientSlug={patientSlug} />;
    case "symptom":
      return <SymptomForm onCancel={onCancel} onReady={onReady} patientSlug={patientSlug} />;
    case "food":
      return <FoodForm onCancel={onCancel} onReady={onReady} patientSlug={patientSlug} />;
    case "medication":
      return <MedicationForm onCancel={onCancel} onReady={onReady} patientSlug={patientSlug} />;
    case "lab":
      return <LabForm onCancel={onCancel} onReady={onReady} patientSlug={patientSlug} />;
    case "imaging":
      return <ImagingForm onCancel={onCancel} onReady={onReady} patientSlug={patientSlug} />;
  }
}

// --- Per-type forms ---------------------------------------------------------

function FormShell({
  children,
  onCancel,
  submitLabel = "Continue",
  onSubmit,
}: {
  children: React.ReactNode;
  onCancel: () => void;
  submitLabel?: string;
  onSubmit: (e: FormEvent) => void;
}) {
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit(e);
      }}
    >
      <div className="modal-body">{children}</div>
      <div className="modal-foot">
        <button type="button" className="btn" onClick={onCancel}>
          Cancel
        </button>
        <button type="submit" className="btn btn-primary">
          {submitLabel}
        </button>
      </div>
    </form>
  );
}

function NoteForm({
  onCancel,
  onReady,
  patientSlug,
  author,
}: {
  onCancel: () => void;
  onReady: FormProps["onReady"];
  patientSlug: string;
  author: string;
}) {
  const [text, setText] = useState("");
  return (
    <FormShell
      onCancel={onCancel}
      onSubmit={() => {
        if (!text.trim()) return;
        onReady(() => {
          submitNote(patientSlug, text.trim(), author);
        });
      }}
    >
      <label className="field">
        Note text
        <textarea value={text} onChange={(e) => setText(e.target.value)} placeholder="Visit summary, assessment, plan…" />
      </label>
    </FormShell>
  );
}

function VitalForm({
  onCancel,
  onReady,
  patientSlug,
}: {
  onCancel: () => void;
  onReady: FormProps["onReady"];
  patientSlug: string;
}) {
  const [channel, setChannel] = useState("bp_systolic");
  const [value, setValue] = useState("");
  const [unit, setUnit] = useState("mmHg");
  return (
    <FormShell
      onCancel={onCancel}
      onSubmit={() => {
        const n = parseFloat(value);
        if (isNaN(n)) return;
        onReady(() => {
          submitVital(patientSlug, channel, n, unit);
        });
      }}
    >
      <label className="field">
        Channel
        <select value={channel} onChange={(e) => setChannel(e.target.value)}>
          <option value="bp_systolic">BP systolic</option>
          <option value="bp_diastolic">BP diastolic</option>
          <option value="hr">Heart rate</option>
          <option value="temp_c">Temperature (°C)</option>
          <option value="spo2">SpO2</option>
          <option value="glucose_mg_dl">Glucose</option>
        </select>
      </label>
      <label className="field">
        Value
        <input type="number" step="any" value={value} onChange={(e) => setValue(e.target.value)} />
      </label>
      <label className="field">
        Unit
        <input type="text" value={unit} onChange={(e) => setUnit(e.target.value)} />
      </label>
    </FormShell>
  );
}

function SymptomForm({
  onCancel,
  onReady,
  patientSlug,
}: {
  onCancel: () => void;
  onReady: FormProps["onReady"];
  patientSlug: string;
}) {
  const [text, setText] = useState("");
  const [severity, setSeverity] = useState<1 | 2 | 3 | 4 | 5>(3);
  return (
    <FormShell
      onCancel={onCancel}
      onSubmit={() => {
        if (!text.trim()) return;
        onReady(() => {
          submitSymptom(patientSlug, text.trim(), severity);
        });
      }}
    >
      <label className="field">
        Description
        <input type="text" value={text} onChange={(e) => setText(e.target.value)} />
      </label>
      <label className="field">
        Severity (1–5)
        <select
          value={severity}
          onChange={(e) => setSeverity(Number(e.target.value) as 1 | 2 | 3 | 4 | 5)}
        >
          {[1, 2, 3, 4, 5].map((n) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
      </label>
    </FormShell>
  );
}

function FoodForm({
  onCancel,
  onReady,
  patientSlug,
}: {
  onCancel: () => void;
  onReady: FormProps["onReady"];
  patientSlug: string;
}) {
  const [text, setText] = useState("");
  const [kcal, setKcal] = useState("");
  return (
    <FormShell
      onCancel={onCancel}
      onSubmit={() => {
        if (!text.trim()) return;
        const k = kcal ? parseFloat(kcal) : undefined;
        onReady(() => {
          submitFood(patientSlug, text.trim(), Number.isFinite(k) ? k : undefined);
        });
      }}
    >
      <label className="field">
        Description
        <input type="text" value={text} onChange={(e) => setText(e.target.value)} />
      </label>
      <label className="field">
        Approx. kcal (optional)
        <input type="number" value={kcal} onChange={(e) => setKcal(e.target.value)} />
      </label>
    </FormShell>
  );
}

function MedicationForm({
  onCancel,
  onReady,
  patientSlug,
}: {
  onCancel: () => void;
  onReady: FormProps["onReady"];
  patientSlug: string;
}) {
  const [name, setName] = useState("");
  const [dose, setDose] = useState("");
  const [schedule, setSchedule] = useState("");
  return (
    <FormShell
      onCancel={onCancel}
      onSubmit={() => {
        if (!name.trim() || !dose.trim()) return;
        onReady(() => {
          submitMedication(patientSlug, name.trim(), dose.trim(), schedule.trim() || "as needed");
        });
      }}
    >
      <label className="field">
        Medication name
        <input type="text" value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      <label className="field">
        Dose
        <input type="text" placeholder="e.g. 500 mg" value={dose} onChange={(e) => setDose(e.target.value)} />
      </label>
      <label className="field">
        Schedule
        <input type="text" placeholder="e.g. 2× daily with meals" value={schedule} onChange={(e) => setSchedule(e.target.value)} />
      </label>
    </FormShell>
  );
}

function LabForm({
  onCancel,
  onReady,
  patientSlug,
}: {
  onCancel: () => void;
  onReady: FormProps["onReady"];
  patientSlug: string;
}) {
  const [panel, setPanel] = useState("");
  const [values, setValues] = useState("");
  return (
    <FormShell
      onCancel={onCancel}
      onSubmit={() => {
        if (!panel.trim()) return;
        onReady(() => {
          submitLab(patientSlug, panel.trim(), values.trim() || "(see attached)");
        });
      }}
    >
      <label className="field">
        Panel name
        <input type="text" value={panel} onChange={(e) => setPanel(e.target.value)} />
      </label>
      <label className="field">
        Values / summary
        <textarea value={values} onChange={(e) => setValues(e.target.value)} />
      </label>
    </FormShell>
  );
}

function ImagingForm({
  onCancel,
  onReady,
  patientSlug,
}: {
  onCancel: () => void;
  onReady: FormProps["onReady"];
  patientSlug: string;
}) {
  const [modality, setModality] = useState("X-ray");
  const [region, setRegion] = useState("");
  const [findings, setFindings] = useState("");
  return (
    <FormShell
      onCancel={onCancel}
      onSubmit={() => {
        if (!region.trim() || !findings.trim()) return;
        onReady(() => {
          submitImaging(patientSlug, modality, region.trim(), findings.trim());
        });
      }}
    >
      <label className="field">
        Modality
        <select value={modality} onChange={(e) => setModality(e.target.value)}>
          <option>X-ray</option>
          <option>CT</option>
          <option>MRI</option>
          <option>US</option>
        </select>
      </label>
      <label className="field">
        Region
        <input type="text" value={region} onChange={(e) => setRegion(e.target.value)} />
      </label>
      <label className="field">
        Findings
        <textarea value={findings} onChange={(e) => setFindings(e.target.value)} />
      </label>
    </FormShell>
  );
}
