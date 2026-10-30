// SPDX-License-Identifier: GPL-3.0-or-later

// Categories: which downloads belong where. The first one that matches, from the top, wins.

import { useEffect, useState } from "react";
import type { CategorySummary } from "../../../protocol/ts/CategorySummary";
import { api, errorText } from "../lib/api";
import { categoryIcon, categoryTint } from "../lib/categoryLook";
import { Button, ChipsField, Field, FolderField, inputClass } from "./Fields";
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

  const position = typeof selected === "number" ? categories.findIndex((c) => c.categoryId === selected) : -1;
  const icon = categoryIcon(selected === "new" ? draft.name : (current?.name ?? ""));

  return (
    <div className="flex gap-5">
      <div className="w-[236px] shrink-0 space-y-2">
        <ul aria-label="Categories" className="space-y-0.5 rounded-xl border border-white/[0.08] bg-white/[0.02] p-1">
          {categories.map((c, i) => {
            const chosen = selected === c.categoryId;
            const look = categoryIcon(c.name);
            return (
              <li
                key={c.categoryId}
                className={`group flex h-[38px] items-center rounded-[9px] border ${
                  chosen ? "border-[#6b88ff]/30 bg-[#4a6cff]/[0.12]" : "border-transparent hover:bg-white/[0.04]"
                }`}
              >
                <button
                  type="button"
                  aria-current={chosen ? "true" : undefined}
                  onClick={() => setSelected(c.categoryId)}
                  className="flex h-full min-w-0 flex-1 items-center gap-2.5 pl-2.5 text-left text-[13px]"
                >
                  <span className={`flex size-6 shrink-0 items-center justify-center rounded-md ${categoryTint(look)}`}>
                    <Icon name={look} className="size-3.5" />
                  </span>
                  <span className={`truncate ${chosen ? "font-semibold text-[#f4f6fb]" : "font-medium text-[#c9cfdd]"}`}>
                    {c.name}
                  </span>
                </button>
                <span className={`flex pr-1 ${chosen ? "" : "invisible group-hover:visible"}`}>
                  <button
                    type="button"
                    aria-label={`Move ${c.name} up`}
                    disabled={i === 0}
                    onClick={() => move(c.categoryId, -1)}
                    className="rounded-md p-1 text-[#8b93a9] hover:bg-white/[0.08] hover:text-[#f4f6fb] disabled:invisible"
                  >
                    <Icon name="up" className="size-3.5" />
                  </button>
                  <button
                    type="button"
                    aria-label={`Move ${c.name} down`}
                    disabled={i === categories.length - 1}
                    onClick={() => move(c.categoryId, 1)}
                    className="rounded-md p-1 text-[#8b93a9] hover:bg-white/[0.08] hover:text-[#f4f6fb] disabled:invisible"
                  >
                    <Icon name="down" className="size-3.5" />
                  </button>
                </span>
              </li>
            );
          })}
          {selected === "new" && (
            <li className="flex h-[38px] items-center gap-2.5 rounded-[9px] border border-[#6b88ff]/30 bg-[#4a6cff]/[0.12] px-2.5 text-[13px] font-semibold text-[#f4f6fb] italic">
              <span className={`flex size-6 items-center justify-center rounded-md ${categoryTint(icon)}`}>
                <Icon name={icon} className="size-3.5" />
              </span>
              <span className="truncate">{draft.name.trim() || "New category"}</span>
            </li>
          )}
        </ul>
        <button
          type="button"
          onClick={() => setSelected("new")}
          disabled={selected === "new"}
          className="group flex h-9 w-full items-center justify-center gap-2 rounded-[10px] border border-dashed border-white/[0.16] text-[13px] font-medium text-[#c9cfdd] hover:border-[#6b88ff]/50 hover:bg-[#4a6cff]/[0.08] hover:text-[#f4f6fb] disabled:pointer-events-none disabled:opacity-40"
        >
          <Icon name="plus" className="size-4 text-[#8ea4ff] transition-[rotate] duration-300 ease-out motion-safe:group-hover:rotate-90" />
          Add category
        </button>
      </div>

      {selected == null ? (
        <p className="pt-2 text-[13px] text-[#8b93a9]">No categories. Add one to sort downloads into folders.</p>
      ) : (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            save();
          }}
          className="flex min-w-0 flex-1 flex-col gap-4 rounded-xl border border-white/[0.08] bg-white/[0.02] p-5"
        >
          <div className="flex items-center gap-3">
            <span className={`flex size-9 shrink-0 items-center justify-center rounded-[10px] ${categoryTint(icon)}`}>
              <Icon name={icon} className="size-[18px]" />
            </span>
            <div className="min-w-0">
              <p className="truncate text-base font-semibold text-[#f4f6fb]">
                {selected === "new" ? draft.name.trim() || "New category" : current?.name}
              </p>
              <p className="text-xs text-[#8b93a9]">
                {selected === "new"
                  ? "Checked after the others, until you move it up."
                  : `Checked ${position + 1} of ${categories.length}, from the top.`}
              </p>
            </div>
          </div>
          <div className="flex gap-3">
            <div className="w-[180px] shrink-0">
              <Field label="Name" htmlFor="cat-name">
                <input id="cat-name" value={draft.name} onChange={(e) => set("name")(e.target.value)} className={inputClass} />
              </Field>
            </div>
            <div className="min-w-0 flex-1">
              <Field label="File extensions" htmlFor="cat-ext">
                <ChipsField
                  id="cat-ext"
                  values={split(draft.extensions)}
                  onChange={(words) => set("extensions")(words.join(", "))}
                  placeholder="zip, rar, 7z"
                  clean={(word) => word.replace(/^\.+/, "").toLowerCase()}
                />
              </Field>
            </div>
          </div>
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
            <p role="alert" className="text-[13px] text-[#ff9a9e]">
              {error}
            </p>
          )}
          <div className="flex items-center gap-2 border-t border-white/[0.05] pt-4">
            {typeof selected === "number" &&
              (confirmDelete ? (
                <Button danger onClick={remove} onBlur={() => setConfirmDelete(false)}>
                  <Icon name="trash" className="size-4" />
                  Click again to delete
                </Button>
              ) : (
                <Button ghost onClick={remove} className="-ml-2 text-[#ff9a9e]! hover:text-[#ffc9cb]!">
                  <Icon name="trash" className="size-4" />
                  Delete category
                </Button>
              ))}
            {selected === "new" && (
              <Button ghost danger className="-ml-2" onClick={() => setSelected(categories[0]?.categoryId ?? null)}>
                Cancel
              </Button>
            )}
            {saved && (
              <span className="ml-auto flex items-center gap-1.5 text-xs text-[#7fe6c4]">
                <Icon name="check" className="size-3.5" />
                Saved
              </span>
            )}
            <Button type="submit" primary className={saved ? "ml-2" : "ml-auto"}>
              {selected === "new" ? "Add category" : "Save changes"}
            </Button>
          </div>
          <p className="-mt-1 text-xs text-[#6b7390]">
            Deleting a category keeps its downloads and their files; they just have no category.
          </p>
        </form>
      )}
    </div>
  );
}
