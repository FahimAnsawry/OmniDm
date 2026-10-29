// SPDX-License-Identifier: GPL-3.0-or-later

// Form pieces shared by the Add download dialog and the capture prompt.

import type { ButtonHTMLAttributes, ReactNode } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Icon } from "./Icon";

export const inputClass =
  "w-full min-w-0 rounded-md border border-neutral-300 bg-white px-2.5 py-1.5 text-[13px] placeholder:text-neutral-400 focus:border-sky-500 focus:ring-2 focus:ring-sky-500/30 focus:outline-none dark:border-neutral-700 dark:bg-neutral-950";

/** A label above its control. */
export function Field({ label, htmlFor, children }: { label: string; htmlFor: string; children: ReactNode }) {
  return (
    <div className="space-y-1">
      <label htmlFor={htmlFor} className="block text-xs font-medium text-neutral-600 dark:text-neutral-400">
        {label}
      </label>
      {children}
    </div>
  );
}

/** A folder path with a Browse button. */
export function FolderField({
  id,
  value,
  onChange,
}: {
  id: string;
  value: string;
  onChange: (folder: string) => void;
}) {
  const browse = async () => {
    const picked = await open({ directory: true, defaultPath: value || undefined, title: "Save in" });
    if (typeof picked === "string") onChange(picked);
  };
  return (
    <div className="flex gap-2">
      <input
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        spellCheck={false}
        className={inputClass}
      />
      <Button type="button" onClick={browse}>
        <Icon name="folder" className="size-4 opacity-70" />
        Browse
      </Button>
    </div>
  );
}

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & { primary?: boolean };

export function Button({ primary, className = "", ...props }: ButtonProps) {
  const look = primary
    ? "border-sky-600 bg-sky-600 text-white hover:bg-sky-700 dark:border-sky-500 dark:bg-sky-600 dark:hover:bg-sky-500"
    : "border-neutral-300 hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800";
  return (
    <button
      type="button"
      {...props}
      className={`flex shrink-0 items-center gap-1.5 rounded-md border px-3.5 py-1.5 text-[13px] font-medium disabled:pointer-events-none disabled:opacity-40 ${look} ${className}`}
    />
  );
}

export const selectClass =
  "rounded-md border border-neutral-300 bg-white px-2 py-1.5 text-[13px] focus:border-sky-500 focus:ring-2 focus:ring-sky-500/30 focus:outline-none dark:border-neutral-700 dark:bg-neutral-950";
