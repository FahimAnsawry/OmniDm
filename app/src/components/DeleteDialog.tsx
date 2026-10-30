// SPDX-License-Identifier: GPL-3.0-or-later

import { useEffect, useRef, useState } from "react";
import { closeDialog } from "../lib/closeDialog";
import { Button } from "./Fields";
import { Icon } from "./Icon";

/** Asks before removing downloads; the files stay unless the box is ticked. */
export function DeleteDialog({
  names,
  onConfirm,
  onCancel,
}: {
  names: string[];
  onConfirm: (deleteFiles: boolean) => void;
  onCancel: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [deleteFiles, setDeleteFiles] = useState(false);

  useEffect(() => {
    dialog.current?.showModal();
  }, []);

  const one = names.length === 1;
  const title = one ? "Remove this download from the list?" : `Remove ${names.length} downloads from the list?`;
  /** Which downloads: the name, or the first few. */
  const which = one ? names[0] : names.slice(0, 3).join(", ") + (names.length > 3 ? ` and ${names.length - 3} more` : "");

  return (
    <dialog
      ref={dialog}
      aria-labelledby="delete-title"
      aria-describedby="delete-which"
      onCancel={(e) => {
        e.preventDefault();
        closeDialog(dialog.current, onCancel);
      }}
      className="m-auto w-[440px] max-w-[calc(100vw-2rem)] rounded-2xl border border-white/[0.12] bg-[#12172a] p-0 font-sans text-[#f4f6fb] shadow-[0_32px_64px_-12px_rgb(0_0_0/0.75)] backdrop:bg-[#04060c]/70"
    >
      <form
        method="dialog"
        onSubmit={(e) => {
          e.preventDefault();
          closeDialog(dialog.current, () => onConfirm(deleteFiles));
        }}
        className="flex flex-col gap-4 p-5"
      >
        <div className="flex items-start gap-3">
          <span className="flex size-10 shrink-0 items-center justify-center rounded-xl border border-[#e5484d]/30 bg-[#e5484d]/15 text-[#ff9a9e]">
            <Icon name="trash" className="size-[18px]" />
          </span>
          <div className="min-w-0 flex-1 pt-0.5">
            <h2 id="delete-title" className="text-base leading-snug font-semibold">
              {title}
            </h2>
            <p id="delete-which" className="mt-1 text-xs break-words text-[#9aa3b8]">
              {which}
            </p>
          </div>
        </div>
        <label
          className={`flex cursor-pointer items-start gap-3 rounded-xl border p-3.5 transition-colors ${
            deleteFiles ? "border-[#e5484d]/25 bg-[#e5484d]/[0.08]" : "border-white/[0.08] bg-white/[0.02] hover:bg-white/[0.04]"
          }`}
        >
          <input
            type="checkbox"
            checked={deleteFiles}
            onChange={(e) => setDeleteFiles(e.target.checked)}
            className="peer sr-only"
          />
          <span
            aria-hidden="true"
            className={`mt-px flex size-[18px] shrink-0 items-center justify-center rounded-[5px] border peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-[#6b88ff] ${
              deleteFiles ? "border-[#e5484d] bg-[#e5484d] text-white" : "border-white/25 bg-white/[0.04]"
            }`}
          >
            {deleteFiles && <Icon name="check" className="size-3" strokeWidth={3} />}
          </span>
          <span className="text-[13px]">
            <span className="block font-semibold">Also delete {one ? "the file" : "the files"} from the disk</span>
            <span className="mt-0.5 block text-xs text-[#9aa3b8]">Unfinished downloads always lose their partial data.</span>
          </span>
        </label>
        <div className="flex justify-end gap-2 pt-1">
          <Button ghost danger onClick={() => closeDialog(dialog.current, onCancel)}>
            Cancel
          </Button>
          <Button type="submit" danger autoFocus>
            <Icon name="trash" className="size-4" />
            {deleteFiles ? "Delete" : "Remove"}
          </Button>
        </div>
      </form>
    </dialog>
  );
}
