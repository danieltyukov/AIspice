import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createMockBackend } from "../ipc/mock";
import { renderWithStore } from "../test/render";
import { fitPanes } from "./ProjectView";
import { Findings } from "./schematic/Findings";
import { SchematicView } from "./schematic/SchematicView";
import { Sidebar } from "./Sidebar";
import { Splitter } from "./Splitter";
import { SpecTable, limitText, marginText } from "./SpecTable";
import { Welcome } from "./Welcome";

describe("Welcome", () => {
  it("lists recent projects and the environment with fixes for what is missing", async () => {
    await renderWithStore(<Welcome />);
    expect(screen.getByRole("heading", { name: "aispice" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /sensor-frontend/ })).toBeInTheDocument();
    expect(await screen.findByText("Optional. Install Xyce 7.9 or newer for large transient runs.")).toBeInTheDocument();
    expect(screen.getAllByText("Add a key in Settings.").length).toBeGreaterThan(0);
    expect(screen.getByText("Key in system keychain")).toBeInTheDocument();
  });

  it("opens a recent project", async () => {
    const { store } = await renderWithStore(<Welcome />);
    fireEvent.click(screen.getByRole("button", { name: /ee2-labs/ }));
    await waitFor(() => expect(store.get().project?.name).toBe("ee2-labs"));
  });
});

describe("Sidebar", () => {
  it("filters circuits and creates a new one", async () => {
    const { store } = await renderWithStore(<Sidebar width={240} />, { project: true });
    fireEvent.change(screen.getByLabelText("Filter circuits"), { target: { value: "amp" } });
    expect(screen.getByRole("button", { name: /ce_amp/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /rc_lowpass/ })).toBeNull();
    fireEvent.change(screen.getByLabelText("Filter circuits"), { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "New circuit" }));
    fireEvent.change(screen.getByLabelText("New circuit name"), { target: { value: "bad name!" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(screen.getByText(/Letters, digits/)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("New circuit name"), { target: { value: "buck" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(store.get().active).toBe("buck.asc"));
    expect(await screen.findByRole("button", { name: /buck.asc/ })).toHaveAttribute("aria-current", "true");
  });
});

describe("SchematicView", () => {
  it("injects the sanitised drawing and lists findings that locate their part", async () => {
    const backend = createMockBackend({ fast: true });
    const read = backend.readCircuit.bind(backend);
    backend.readCircuit = async (path, hl) => {
      const v = await read(path, hl);
      return { ...v, svg: v.svg.replace("</svg>", '<script>window.__pwned = 1</script><g onclick="x()" data-inst="X9"></g></svg>') };
    };
    const { store, container } = await renderWithStore(<SchematicView />, { backend, project: true });
    await store.selectCircuit("opamp_inverting.asc");
    await waitFor(() => expect(container.querySelector('[data-inst="RL"]')).not.toBeNull());
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("[onclick]")).toBeNull();
    const finding = await screen.findByRole("button", { name: /Net N004 reaches only one pin/ });
    fireEvent.click(finding);
    expect(store.get().locate?.inst).toBe("RL");
    expect(await screen.findByRole("dialog", { name: "RL details" })).toHaveTextContent("N004");
  });

  it("zooms with the keyboard and fits again with F", async () => {
    await renderWithStore(<SchematicView />, { project: true });
    const pane = await screen.findByTestId("schematic-pane");
    const level = () => screen.getByTestId("zoom-level").textContent;
    const start = level();
    fireEvent.keyDown(pane, { key: "+" });
    expect(level()).not.toBe(start);
    fireEvent.keyDown(pane, { key: "f" });
    expect(level()).toBe(start);
  });
});

describe("Findings", () => {
  it("summarises and labels severity in words", () => {
    const onLocate = vi.fn();
    render(
      <Findings
        findings={[
          { severity: "error", rule: "no-ground", message: "No ground." },
          { severity: "warning", rule: "floating-pin", message: "R1.B floats.", parts: ["R1"] },
        ]}
        onLocate={onLocate}
      />,
    );
    expect(screen.getByText("1 error, 1 warning")).toBeInTheDocument();
    expect(screen.getByText("Error")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /R1.B floats/ }));
    expect(onLocate).toHaveBeenCalledWith("R1");
  });
});

describe("Splitter", () => {
  it("resizes with arrows, Home, End and resets on Enter", () => {
    const onChange = vi.fn();
    const onReset = vi.fn();
    render(<Splitter label="Resize chat" orientation="vertical" invert value={400} min={300} max={600} onChange={onChange} onReset={onReset} />);
    const sep = screen.getByRole("separator", { name: "Resize chat" });
    expect(sep).toHaveAttribute("aria-valuenow", "400");
    fireEvent.keyDown(sep, { key: "ArrowLeft" });
    expect(onChange).toHaveBeenLastCalledWith(416);
    fireEvent.keyDown(sep, { key: "ArrowRight", shiftKey: true });
    expect(onChange).toHaveBeenLastCalledWith(336);
    fireEvent.keyDown(sep, { key: "End" });
    expect(onChange).toHaveBeenLastCalledWith(600);
    fireEvent.keyDown(sep, { key: "Enter" });
    expect(onReset).toHaveBeenCalled();
  });
});

describe("fitPanes", () => {
  it("keeps the centre usable by shrinking the chat first", () => {
    expect(fitPanes(240, 400, 1440)).toEqual({ sidebar: 240, chat: 400 });
    const narrow = fitPanes(240, 500, 1024);
    expect(narrow.chat).toBe(1024 - 380 - 2 - 240);
    expect(fitPanes(400, 700, 1440)).toEqual({ sidebar: 360, chat: 620 });
    expect(fitPanes(360, 620, 900)).toEqual({ sidebar: 218, chat: 300 });
  });
});

describe("SpecTable", () => {
  it("writes limits and signed margins in the spec's unit", () => {
    const row = { name: "fc", value: 1005, min: 900, max: 1100, pass: true, margin: 95, display: "1.005 kHz" };
    expect(limitText(row)).toBe("900 Hz to 1.10 kHz");
    expect(marginText(row)).toBe("+95.0 Hz");
    expect(marginText({ ...row, margin: -5.8, display: "14.20 dB" })).toBe("-5.80 dB");
    render(<SpecTable report={{ rows: [row], all_pass: true, summary: "All 1 specs pass" }} />);
    expect(screen.getByText("Pass")).toBeInTheDocument();
  });
});
