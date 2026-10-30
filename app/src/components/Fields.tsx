// SPDX-License-Identifier: GPL-3.0-or-later

// Form pieces shared by the Add download dialog, Settings and the details window.

import { type ButtonHTMLAttributes, type ReactNode, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Icon } from "./Icon";

/** A field's box, without its text: the folder field wraps an input in one. */
const boxClass =
  "rounded-[10px] border border-white/10 bg-white/[0.04] transition-colors hover:border-white/[0.16] focus-within:border-[#6b88ff] focus-within:shadow-[0_0_0_3px_rgb(74_108_255/0.25)]";

/** A text box without its height and side padding, for one that sets its own. */
export const inputBase = `w-full min-w-0 text-[13px] text-[#f4f6fb] placeholder:text-[#6b7390] focus:outline-none ${boxClass}`;

export const inputClass = `h-9 px-3 ${inputBase}`;

/** A drop-down without its height. */
export const selectBase = `select-chevron cursor-pointer appearance-none pr-8 pl-3 text-[13px] font-medium text-[#f4f6fb] focus:outline-none ${boxClass}`;

export const selectClass = `h-9 ${selectBase}`;

/** A label above its control. */
export function Field({ label, htmlFor, children }: { label: ReactNode; htmlFor: string; children: ReactNode }) {
  return (
    <div className="space-y-1.5">
      <label htmlFor={htmlFor} className="block text-xs font-medium text-[#c9cfdd]">
        {label}
      </label>
      {children}
    </div>
  );
}

/** A folder path with a Browse button, in one box. */
export function FolderField({
  id,
  value,
  onChange,
  placeholder,
  onBlur,
  tall = false,
  className = "",
}: {
  id: string;
  value: string;
  /** `picked` is true for a folder chosen with Browse (not typed). */
  onChange: (folder: string, picked?: boolean) => void;
  placeholder?: string;
  onBlur?: () => void;
  /** 40px high, as in the Add download dialog. */
  tall?: boolean;
  className?: string;
}) {
  const browse = async () => {
    const picked = await open({ directory: true, defaultPath: value || undefined, title: "Save in" });
    if (typeof picked === "string") onChange(picked, true);
  };
  return (
    <div className={`flex items-center gap-2 pr-1 pl-3 ${tall ? "h-10" : "h-9"} ${boxClass} ${className}`}>
      <Icon name="folder" className="size-4 shrink-0 text-[#ffb38a]" />
      <input
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onBlur}
        placeholder={placeholder}
        spellCheck={false}
        className="h-full min-w-0 flex-1 bg-transparent text-[13px] text-[#f4f6fb] placeholder:text-[#6b7390] focus:outline-none"
      />
      <button
        type="button"
        onClick={browse}
        className="h-7 shrink-0 rounded-[7px] bg-white/[0.07] px-2.5 text-[12.5px] font-semibold text-[#f4f6fb] hover:bg-white/[0.12]"
      >
        Browse
      </button>
    </div>
  );
}

/**
 * Words as chips, like "mp4 mkv webm". Enter, a comma, a space or leaving the box turns
 * what was typed into chips; Backspace in an empty box takes the last one back to edit.
 */
export function ChipsField({
  id,
  values,
  onChange,
  placeholder,
  clean = (word) => word,
}: {
  id: string;
  values: string[];
  onChange: (values: string[]) => void;
  placeholder?: string;
  /** Tidies a typed word, e.g. ".MP4" → "mp4"; an empty result is dropped. */
  clean?: (word: string) => string;
}) {
  const [text, setText] = useState("");

  const commit = (typed: string) => {
    const words = typed
      .split(/[\s,;]+/)
      .map(clean)
      .filter((w) => w && !values.includes(w));
    if (words.length > 0) onChange([...values, ...new Set(words)]);
    setText("");
  };

  return (
    <div
      onClick={(e) => e.currentTarget.querySelector("input")?.focus()}
      className={`flex min-h-9 cursor-text flex-wrap items-center gap-1.5 px-1.5 py-1 ${boxClass}`}
    >
      {values.map((word) => (
        <span
          key={word}
          className="flex h-6 items-center gap-0.5 rounded-md bg-white/[0.08] pr-0.5 pl-2 text-xs font-medium text-[#f4f6fb]"
        >
          {word}
          <button
            type="button"
            aria-label={`Remove ${word}`}
            onClick={() => onChange(values.filter((w) => w !== word))}
            className="flex size-5 items-center justify-center rounded text-[#8b93a9] hover:bg-white/10 hover:text-[#f4f6fb]"
          >
            <Icon name="close" className="size-3" />
          </button>
        </span>
      ))}
      <input
        id={id}
        value={text}
        onChange={(e) => {
          const typed = e.target.value;
          // A comma or space ends a word.
          if (/[\s,;]/.test(typed)) commit(typed);
          else setText(typed);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && text.trim()) {
            e.preventDefault();
            commit(text);
          } else if (e.key === "Backspace" && !text && values.length > 0) {
            e.preventDefault();
            setText(values[values.length - 1]);
            onChange(values.slice(0, -1));
          }
        }}
        onBlur={() => commit(text)}
        onPaste={(e) => {
          e.preventDefault();
          commit(text + " " + e.clipboardData.getData("text"));
        }}
        placeholder={values.length === 0 ? placeholder : undefined}
        spellCheck={false}
        className="h-6 min-w-16 flex-1 bg-transparent px-1.5 text-[13px] text-[#f4f6fb] placeholder:text-[#6b7390] focus:outline-none"
      />
    </div>
  );
}

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  primary?: boolean;
  /** Red: the button removes something. */
  danger?: boolean;
  /** No box until hovered. */
  ghost?: boolean;
};

export function Button({ primary, danger, ghost, className = "", ...props }: ButtonProps) {
  const look = primary
    ? "button-primary"
    : danger
      ? "button-danger"
      : ghost
        ? "border border-transparent text-[#c9cfdd] hover:bg-white/[0.06] hover:text-[#f4f6fb]"
        : "border border-white/[0.12] bg-white/[0.05] text-[#f4f6fb] hover:bg-white/[0.09]";
  return (
    <button
      type="button"
      {...props}
      className={`flex h-9 shrink-0 items-center justify-center gap-2 rounded-[10px] px-4 text-[13px] font-semibold transition focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#6b88ff] active:scale-[0.98] disabled:pointer-events-none disabled:opacity-40 ${look} ${className}`}
    />
  );
}
