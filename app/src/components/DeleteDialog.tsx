// SPDX-License-Identifier: GPL-3.0-or-later

import { useEffect, useRef, useState } from "react";

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

  const title =
    names.length === 1 ? `Remove "${names[0]}" from the list?` : `Remove ${names.length} downloads from the list?`;

  return (
    <dialog
      ref={dialog}
      aria-labelledby="delete-title"
      onCancel={(e) => {
        e.preventDefault();
        onCancel();
      }}
      className="m-auto w-[26rem] max-w-[calc(100vw-2rem)] rounded-xl border border-neutral-200 bg-white p-0 text-neutral-900 shadow-2xl backdrop:bg-black/30 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
    >
      <form
        method="dialog"
        onSubmit={(e) => {
          e.preventDefault();
          onConfirm(deleteFiles);
        }}
        className="space-y-4 p-5"
      >
        <h2 id="delete-title" className="text-[15px] font-semibold break-words">
          {title}
        </h2>
        <label className="flex items-start gap-2.5 text-[13px]">
          <input
            type="checkbox"
            checked={deleteFiles}
            onChange={(e) => setDeleteFiles(e.target.checked)}
            className="mt-0.5 size-4 accent-red-600"
          />
          <span>
            Also delete {names.length === 1 ? "the file" : "the files"} from the disk
            <span className="block text-neutral-500">Unfinished downloads always lose their partial data.</span>
          </span>
        </label>
        <div className="flex justify-end gap-2 pt-1">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-md border border-neutral-300 px-3.5 py-1.5 text-[13px] font-medium hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800"
          >
            Cancel
          </button>
          <button
            type="submit"
            autoFocus
            className="rounded-md bg-red-600 px-3.5 py-1.5 text-[13px] font-medium text-white hover:bg-red-700"
          >
            {deleteFiles ? "Delete" : "Remove"}
          </button>
        </div>
      </form>
    </dialog>
  );
}
