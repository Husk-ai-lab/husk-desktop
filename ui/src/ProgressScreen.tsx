import { Check } from "./Icons";
import { Illustration } from "./Illustration";

export type StepState = "done" | "now" | "todo";

/** "Discovery — in corso" layout: copy and progress on the left, line-art illustration on the right. */
export function ProgressScreen(props: {
  eyebrow: string;
  title: string;
  lead?: string;
  headLabel: string;
  percent: number | null;
  steps: { label: string; state: StepState }[];
}) {
  return (
    <div className="split">
      <div className="copy">
        <p className="eyebrow">{props.eyebrow}</p>
        <h1 className="display">{props.title}</h1>
        {props.lead && <p className="lead">{props.lead}</p>}
        <div className="progress-head">
          <h2>{props.headLabel}</h2>
          {props.percent !== null && <span className="pct">{props.percent}%</span>}
        </div>
        <div className={`bar${props.percent === null ? " indeterminate" : ""}`} role="progressbar" aria-valuenow={props.percent ?? undefined}>
          <i style={props.percent !== null ? { width: `${props.percent}%` } : undefined} />
        </div>
        <ul className="steps">
          {props.steps.map((s) => (
            <li key={s.label} className={s.state}>
              <span className="ring">{s.state === "done" && <Check />}</span>
              {s.label}
            </li>
          ))}
        </ul>
      </div>
      <div className="art"><Illustration /></div>
    </div>
  );
}
