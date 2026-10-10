import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { renderWithStore } from "../test/render";
import { SettingsDialog } from "./SettingsDialog";

describe("SettingsDialog", () => {
  it("sets a key without echoing it back, then removes it", async () => {
    const { store } = await renderWithStore(<SettingsDialog />);
    const openai = screen.getByRole("listitem", { name: "OpenAI" });
    expect(within(openai).getByTestId("key-status-openai")).toHaveTextContent("No key");
    fireEvent.click(within(openai).getByRole("button", { name: "Set key" }));
    const field = within(openai).getByLabelText("OpenAI API key");
    expect(field).toHaveAttribute("type", "password");
    fireEvent.change(field, { target: { value: "sk-live-abcdef123456" } });
    fireEvent.click(within(openai).getByRole("button", { name: "Save key" }));
    await waitFor(() => expect(within(openai).getByTestId("key-status-openai")).toHaveTextContent("Key in system keychain"));
    expect(document.body.innerHTML).not.toContain("sk-live-abcdef123456");
    fireEvent.click(within(openai).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(within(openai).getByTestId("key-status-openai")).toHaveTextContent("No key"));
    expect(store.get().keys.find((k) => k.id === "openai")?.configured).toBe(false);
  });

  it("saves edited settings and closes", async () => {
    const { store } = await renderWithStore(<SettingsDialog />);
    store.openSettings();
    fireEvent.click(screen.getByRole("button", { name: "Ask before applying" }));
    fireEvent.click(screen.getByRole("button", { name: "Dark" }));
    fireEvent.change(screen.getByLabelText("Max steps per turn"), { target: { value: "40" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(store.get().settingsOpen).toBe(false));
    const s = store.get().settings!;
    expect(s.edit_mode).toBe("ask");
    expect(s.theme).toBe("dark");
    expect(s.max_steps).toBe(40);
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });

  it("loads the live model list for the chosen provider", async () => {
    await renderWithStore(<SettingsDialog />);
    expect(await screen.findByText("3 models available")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "google" } });
    expect(await screen.findByText(/Add an API key to list/)).toBeInTheDocument();
  });

  it("traps focus inside and closes on Escape", async () => {
    const { store } = await renderWithStore(<SettingsDialog />);
    store.openSettings();
    const dialog = screen.getByRole("dialog", { name: "Settings" });
    const buttons = within(dialog).getAllByRole("button");
    const last = buttons[buttons.length - 1];
    last.focus();
    fireEvent.keyDown(dialog, { key: "Tab" });
    expect(dialog.contains(document.activeElement)).toBe(true);
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(store.get().settingsOpen).toBe(false);
  });
});
