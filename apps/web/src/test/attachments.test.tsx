import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { useState } from "react";
import { render, screen, waitFor, fireEvent, act } from "@testing-library/react";
import { AttachmentField, AttachmentChips, AttachmentCover, formatBytes, type AttachedFile } from "../ui/Attachments";
import { RecordDetailDrawer } from "../RecordDetailDrawer";
import { DataGrid } from "../DataGrid";
import * as api from "../api";
import { ApiError } from "../api";
import type { AppTable, FieldSpec } from "../types";

const png = (name: string) => new File([new Uint8Array(2048)], name, { type: "image/png" });

function meta(over: Partial<api.RecordFile> = {}): api.RecordFile {
  return { ref_id: "r1", field: "doc", name: "a.png", size: 2048, media_type: "image/png", scan_state: "clean", ...over };
}

function Harness({ initial = [] as AttachedFile[], onChange = vi.fn() }) {
  const [value, setValue] = useState<AttachedFile[]>(initial);
  return (
    <AttachmentField
      appSlug="demo"
      recordId="rec1"
      fieldName="doc"
      label="Documents"
      value={value}
      onChange={(next) => {
        onChange(next);
        setValue(next);
      }}
    />
  );
}

describe("AttachmentField", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(api, "listRecordFiles").mockResolvedValue({ fields: ["doc"], files: [] });
  });

  it("shows progress while a file uploads, then a chip, and a refusal shows the specific message", async () => {
    let finish!: (f: api.UploadedFile) => void;
    const upload = vi.spyOn(api, "uploadRecordFile").mockImplementationOnce((_slug, _file, opts) => {
      opts?.onProgress?.(1024, 2048);
      return new Promise((resolve) => {
        finish = resolve;
      });
    });
    const onChange = vi.fn();
    render(<Harness onChange={onChange} />);
    const input = await screen.findByTestId("attachment-input-doc");
    fireEvent.change(input, { target: { files: [png("photo.png")] } });

    expect(await screen.findByText("Uploading photo.png: 50%")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Uploading photo.png" })).toHaveAttribute("aria-valuenow", "50");
    expect(screen.getByRole("button", { name: "Cancel upload of photo.png" })).toBeInTheDocument();
    expect(upload).toHaveBeenCalledWith("demo", expect.any(File), expect.objectContaining({ signal: expect.any(AbortSignal) }));

    await act(async () => finish({ ref_id: "r1", name: "photo.png", size: 2048, media_type: "image/png", scan_state: "skipped" }));
    const list = await screen.findByRole("list", { name: "Documents" });
    expect(list).toHaveTextContent("photo.png");
    expect(list).toHaveTextContent("2 KB");
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(onChange).toHaveBeenCalledWith([{ ref_id: "r1", name: "photo.png", size: 2048, media_type: "image/png" }]);

    vi.spyOn(api, "uploadRecordFile").mockRejectedValueOnce(
      new ApiError(400, "Programs and web pages cannot be uploaded.", { code: "file_type_blocked" }),
    );
    fireEvent.change(input, { target: { files: [png("tool.html")] } });
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("tool.html");
    expect(alert).toHaveTextContent("Programs and web pages cannot be uploaded.");
    expect(alert).toHaveTextContent("Save it as a PDF, an image, or a plain document");
  });

  it("has a drop target and a button that work from the keyboard", async () => {
    const upload = vi.spyOn(api, "uploadRecordFile").mockResolvedValue({ ref_id: "r2", name: "dropped.png", size: 2048, media_type: "image/png", scan_state: "pending" });
    const click = vi.spyOn(HTMLInputElement.prototype, "click").mockImplementation(() => {});
    render(<Harness />);
    const zone = await screen.findByRole("button", { name: /Drop files here/ });
    expect(zone).toHaveAttribute("tabindex", "0");
    fireEvent.keyDown(zone, { key: "Enter" });
    fireEvent.keyDown(zone, { key: " " });
    expect(click).toHaveBeenCalledTimes(2);
    const choose = screen.getByRole("button", { name: "Choose files" });
    expect(choose.tagName).toBe("BUTTON");
    fireEvent.click(choose);
    expect(click).toHaveBeenCalledTimes(3);

    fireEvent.drop(zone, { dataTransfer: { files: [png("dropped.png")] } });
    await waitFor(() => expect(upload).toHaveBeenCalledTimes(1));
    expect(await screen.findByRole("list", { name: "Documents" })).toHaveTextContent("dropped.png");
  });

  it("says in text when a file is not scanned, is scanning, or failed its scan", async () => {
    vi.mocked(api.listRecordFiles).mockResolvedValue({
      fields: ["doc"],
      files: [meta({ ref_id: "a", name: "a.pdf", scan_state: "skipped" }), meta({ ref_id: "b", name: "b.pdf", scan_state: "pending" }), meta({ ref_id: "c", name: "c.pdf", scan_state: "error" }), meta({ ref_id: "d", name: "d.pdf", scan_state: "clean" })],
    });
    const value = ["a", "b", "c", "d"].map((x) => ({ ref_id: x, name: `${x}.pdf`, size: 10, media_type: "application/pdf" }));
    render(<Harness initial={value} />);
    expect(await screen.findByText("Not scanned")).toBeInTheDocument();
    expect(screen.getByText("Scanning")).toBeInTheDocument();
    expect(screen.getByText("Scan failed")).toBeInTheDocument();
    expect(screen.getByText("Scanned")).toBeInTheDocument();
    // A file that is still scanning cannot be downloaded yet.
    expect(screen.getByRole("button", { name: "Download b.pdf" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Download a.pdf" })).toBeEnabled();
  });

  it("renders nothing at all for a field the person may not read", async () => {
    vi.mocked(api.listRecordFiles).mockResolvedValue({ fields: [], files: [] });
    const { container } = render(<Harness />);
    await waitFor(() => expect(api.listRecordFiles).toHaveBeenCalled());
    await act(async () => {});
    expect(container).toBeEmptyDOMElement();
  });

  it("downloads through the authorized route", async () => {
    vi.mocked(api.listRecordFiles).mockResolvedValue({ fields: ["doc"], files: [meta({ scan_state: "clean" })] });
    const fetchFile = vi.spyOn(api, "fetchRecordFile").mockResolvedValue(new Blob(["x"], { type: "image/png" }));
    const url = vi.fn(() => "blob:fake");
    Object.assign(URL, { createObjectURL: url, revokeObjectURL: vi.fn() });
    const anchorClick = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
    render(<Harness initial={[{ ref_id: "r1", name: "a.png", size: 2048, media_type: "image/png" }]} />);
    fireEvent.click(await screen.findByRole("button", { name: "Download a.png" }));
    await waitFor(() => expect(anchorClick).toHaveBeenCalled());
    expect(fetchFile).toHaveBeenCalledWith("demo", "rec1", "r1");
  });
});

const fields: FieldSpec[] = [
  { name: "title", label: "Title", field_type: "Text", required: false, ferpa_sensitive: false },
  { name: "doc", label: "Documents", field_type: "Attachment", required: false, ferpa_sensitive: false },
];

describe("the record drawer", () => {
  afterEach(() => vi.restoreAllMocks());

  it("removes a file from the list and sends the update", async () => {
    vi.spyOn(api, "listRecordFiles").mockResolvedValue({ fields: ["doc"], files: [meta()] });
    const table = {
      id: "t1",
      name: "Things",
      slug: "things",
      fields,
      primary_field: "title",
      records: [{ id: "rec1", title: "One", doc: [{ ref_id: "r1", name: "a.png", size: 2048, media_type: "image/png" }] }],
    } as unknown as AppTable;
    const onUpdate = vi.fn();
    render(
      <RecordDetailDrawer
        selectedRecordId="rec1"
        onClose={() => {}}
        activeTable={table}
        tables={[table]}
        setTables={() => {}}
        appSlug="demo"
        onUpdateRecordField={onUpdate}
        onDuplicateRecord={() => {}}
        onDeleteRecord={() => {}}
      />,
    );
    const list = await screen.findByRole("list", { name: "Documents" });
    expect(list).toHaveTextContent("a.png");
    fireEvent.click(screen.getByRole("button", { name: "Remove a.png" }));
    expect(onUpdate).toHaveBeenCalledWith("rec1", "doc", []);
  });
});

describe("file chips in the grid", () => {
  it("show the name and size of each file as text", () => {
    render(
      <DataGrid
        fields={fields}
        records={[{ id: "rec1", title: "One", doc: [{ ref_id: "r1", name: "a.png", size: 2048, media_type: "image/png" }, { ref_id: "r2", name: "b.pdf", size: 5 * 1024 * 1024, media_type: "application/pdf" }] }]}
        onPatch={() => {}}
      />,
    );
    const chips = screen.getByRole("list", { name: "Files" });
    expect(chips).toHaveTextContent("a.png");
    expect(chips).toHaveTextContent("2 KB");
    expect(chips).toHaveTextContent("b.pdf");
    expect(chips).toHaveTextContent("5 MB");
  });

  it("formats sizes in words", () => {
    expect([formatBytes(0), formatBytes(900), formatBytes(2048), formatBytes(1536 * 1024), formatBytes(5 * 1024 * 1024)]).toEqual(["0 B", "900 B", "2 KB", "1.5 MB", "5 MB"]);
    render(<AttachmentChips value={[]} />);
    expect(screen.queryByRole("list")).not.toBeInTheDocument();
  });
});

describe("the gallery cover", () => {
  afterEach(() => vi.restoreAllMocks());

  it("loads the first image through the authorized route and describes it", async () => {
    Object.assign(URL, { createObjectURL: vi.fn(() => "blob:cover"), revokeObjectURL: vi.fn() });
    const fetchFile = vi.spyOn(api, "fetchRecordFile").mockResolvedValue(new Blob(["x"], { type: "image/png" }));
    render(
      <AttachmentCover
        appSlug="demo"
        recordId="rec1"
        value={[
          { ref_id: "p", name: "notes.pdf", size: 1, media_type: "application/pdf" },
          { ref_id: "i", name: "front.png", size: 1, media_type: "image/png" },
        ]}
      />,
    );
    const img = await screen.findByRole("img", { name: "front.png" });
    expect(img).toHaveAttribute("src", "blob:cover");
    expect(fetchFile).toHaveBeenCalledWith("demo", "rec1", "i");
  });

  it("renders nothing when the record holds no image", () => {
    const { container } = render(<AttachmentCover appSlug="demo" recordId="rec1" value={[{ ref_id: "p", name: "notes.pdf", size: 1, media_type: "application/pdf" }]} />);
    expect(container).toBeEmptyDOMElement();
  });
});
