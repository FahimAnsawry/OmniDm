// SPDX-License-Identifier: GPL-3.0-or-later

// Categories: which downloads belong where. The first one that matches, from the top, wins.

import { useEffect, useState } from "react";
import type { CategorySummary } from "../../../protocol/ts/CategorySummary";
import { api, errorText } from "../lib/api";
import { Button, Field, FolderField, inputClass } from "./Fields";
import { Icon } from "./Icon";

type Draft = { name: string; extensions: string; mimeTypes: string; hosts: string; folder: string };

const EMPTY: Draft = { name: "", extensions: "", mimeTypes: "", hosts: "", folder: "" };

const draftOf = (c: CategorySummary): Draft => ({
  name: c.name,
  extensions: c.extensions.join(", "),
  mimeTypes: c.mimeTypes.join(", "),
  hosts: c.hosts.join(", "),
  folder: c.folder ?? "",
});

/** "zip, rar 7z" → ["zip", "rar", "7z"] */
const split = (text: string) => text.split(/[\s,;]+/).filter(Boolean);

export function CategoriesEditor({
  categories,
  downloadsFolder,
  onChanged,
}: {
  categories: CategorySummary[];
  downloadsFolder: string;
  onChanged: () => void;
}) {
  /** The category being edited; "new" for one not saved yet. */
  const [selected, setSelected] = useState<number | "new" | null>(categories[0]?.categoryId ?? null);
  const [draft, setDraft] = useState<Draft>(EMPTY);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const current = typeof selected === "number" ? categories.find((c) => c.categoryId === selected) : undefined;

  // A new selection starts from its saved values.
  useEffect(() => {
    setDraft(current ? draftOf(current) : EMPTY);
    setError(null);
    setSaved(false);
    setConfirmDelete(false);
    // Only when the selection changes, not on every reload of the list.
  }, [selected]);

  const run = async (action: () => Promise<unknown>) => {
    try {
      await action();
      setError(null);
      onChanged();
      return true;
    } catch (e) {
      setError(errorText(e));
      return false;
    }
  };

  const save = async () => {
    const edit = {
      name: draft.name,
      extensions: split(draft.extensions),
      mimeTypes: split(draft.mimeTypes),
      hosts: split(draft.hosts),
      folder: draft.folder.trim() || undefined,
    };
    if (selected === "new") {
      let id = 0;
      if (await run(async () => (id = await api.createCategory(edit)))) {
        setSelected(id);
      }
    } else if (typeof selected === "number" && (await run(() => api.editCategory(selected, edit)))) {
      setSaved(true);
    }
  };

  const remove = async () => {
    if (typeof selected !== "number") return;
    if (!confirmDelete) {
      setConfirmDelete(true);
      return;
    }
    const index = categories.findIndex((c) => c.categoryId === selected);
    if (await run(() => api.deleteCategory(selected))) {
      const rest = categories.filter((c) => c.categoryId !== selected);
      setSelected(rest[Math.min(index, rest.length - 1)]?.categoryId ?? null);
    }
  };

  const move = (id: number, by: number) => {
    const index = categories.findIndex((c) => c.categoryId === id);
    run(() => api.moveCategory(id, Math.max(0, index + by)));
  };

  const set = (field: keyof Draft) => (value: string) => {
    setDraft((d) => ({ ...d, [field]: value }));
    setSaved(false);
  };

  return (
    <div className="flex gap-5">
      <div className="w-56 shrink-0 space-y-2">
        <p className="text-xs text-neutral-500">The first category that matches a download is used, from the top.</p>
        <ul aria-label="Categories" className="overflow-hidden rounded-lg border border-neutral-200 dark:border-neutral-800">
          {categories.map((c, i) => (
            <li
              key={c.categoryId}
              className={`group flex items-center border-b border-neutral-100 last:border-b-0 dark:border-neutral-900 ${
                selected === c.categoryId ? "bg-sky-100 dark:bg-sky-900/40" : ""
              }`}
            >
              <button
                type="button"
                aria-current={selected === c.categoryId ? "true" : undefined}
                onClick={() => setSelected(c.categoryId)}
                className="flex min-w-0 flex-1 items-center gap-2 px-3 py-2 text-left text-[13px]"
              >
                <Icon name="tag" className="size-3.5 opacity-60" />
                <span className="truncate">{c.name}</span>
              </button>
              <button
                type="button"
                aria-label={`Move ${c.name} up`}
                disabled={i === 0}
                onClick={() => move(c.categoryId, -1)}
                className="p-1.5 text-neutral-500 hover:text-neutral-900 disabled:invisible dark:hover:text-neutral-100"
              >
                <Icon name="up" className="size-3.5" />
              </button>
              <button
                type="button"
                aria-label={`Move ${c.name} down`}
                disabled={i === categories.length - 1}
                onClick={() => move(c.categoryId, 1)}
                className="p-1.5 pr-2 text-neutral-500 hover:text-neutral-900 disabled:invisible dark:hover:text-neutral-100"
              >
                <Icon name="down" className="size-3.5" />
              </button>
            </li>
          ))}
          {selected === "new" && (
            <li className="flex items-center gap-2 bg-sky-100 px-3 py-2 text-[13px] italic dark:bg-sky-900/40">
              <Icon name="tag" className="size-3.5 opacity-60" />
              New category
            </li>
          )}
        </ul>
        <Button onClick={() => setSelected("new")} disabled={selected === "new"}>
          <Icon name="plus" className="size-4" />
          Add category
        </Button>
      </div>

      {selected == null ? (
        <p className="pt-6 text-sm text-neutral-500">No categories. Add one to sort downloads into folders.</p>
      ) : (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            save();
          }}
          className="min-w-0 flex-1 space-y-3.5"
        >
          <Field label="Name" htmlFor="cat-name">
            <input id="cat-name" value={draft.name} onChange={(e) => set("name")(e.target.value)} className={inputClass} />
          </Field>
          <Field label="File extensions" htmlFor="cat-ext">
            <input
              id="cat-ext"
              value={draft.extensions}
              onChange={(e) => set("extensions")(e.target.value)}
              placeholder="zip, rar, 7z"
              spellCheck={false}
              className={inputClass}
            />
          </Field>
          <Field label="File types (what the server says)" htmlFor="cat-mime">
            <input
              id="cat-mime"
              value={draft.mimeTypes}
              onChange={(e) => set("mimeTypes")(e.target.value)}
              placeholder="video/*, application/pdf"
              spellCheck={false}
              className={inputClass}
            />
          </Field>
          <Field label="Sites (with their subdomains)" htmlFor="cat-hosts">
            <input
              id="cat-hosts"
              value={draft.hosts}
              onChange={(e) => set("hosts")(e.target.value)}
              placeholder="example.com"
              spellCheck={false}
              className={inputClass}
            />
          </Field>
          <Field label="Save in" htmlFor="cat-folder">
            <FolderField
              id="cat-folder"
              value={draft.folder}
              onChange={(value) => set("folder")(value)}
              placeholder={`The downloads folder (${downloadsFolder})`}
            />
          </Field>
          {error && (
            <p role="alert" className="text-[13px] text-red-700 dark:text-red-400">
              {error}
            </p>
          )}
          <div className="flex items-center gap-2 pt-1">
            {typeof selected === "number" && (
              <Button
                onClick={remove}
                onBlur={() => setConfirmDelete(false)}
                className={confirmDelete ? "border-red-600 bg-red-600 text-white hover:bg-red-700" : "text-red-700 dark:text-red-400"}
              >
                <Icon name="trash" className="size-4" />
                {confirmDelete ? "Click again to delete" : "Delete"}
              </Button>
            )}
            {selected === "new" && <Button onClick={() => setSelected(categories[0]?.categoryId ?? null)}>Cancel</Button>}
            {saved && <span className="text-xs text-emerald-700 dark:text-emerald-400">Saved</span>}
            <Button type="submit" primary className="ml-auto">
              {selected === "new" ? "Add" : "Save"}
            </Button>
          </div>
          <p className="text-xs text-neutral-500">
            Deleting a category keeps its downloads and their files; they just have no category.
          </p>
        </form>
      )}
    </div>
  );
}
