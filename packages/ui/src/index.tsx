import type { InputHTMLAttributes, ReactNode } from 'react';

export function StatusBadge({
  tone,
  children,
}: {
  tone: 'neutral' | 'working' | 'success' | 'danger';
  children: ReactNode;
}) {
  return <span className={`status-badge status-badge--${tone}`}>{children}</span>;
}

export function ProgressMeter({
  value,
  label,
}: {
  value: number;
  label: string;
}) {
  const normalized = Math.max(0, Math.min(100, value));
  return (
    <div className="progress-meter" aria-label={label}>
      <div className="progress-meter__track">
        <div className="progress-meter__fill" style={{ transform: `scaleX(${normalized / 100})` }} />
      </div>
      <span>{Math.round(normalized)}%</span>
    </div>
  );
}

export function FileInput({
  label,
  helper,
  ...inputProps
}: InputHTMLAttributes<HTMLInputElement> & {
  label: string;
  helper: string;
}) {
  return (
    <label className="file-input">
      <span className="file-input__label">{label}</span>
      <input type="file" {...inputProps} />
      <span className="file-input__helper">{helper}</span>
    </label>
  );
}
