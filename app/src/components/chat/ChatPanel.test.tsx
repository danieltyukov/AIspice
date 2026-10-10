import { fireEvent, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { renderWithStore } from "../../test/render";
import { ChatPanel } from "./ChatPanel";

describe("ChatPanel", () => {
  it("offers suggestions for the open circuit and fills the composer", async () => {
    await renderWithStore(<ChatPanel width={400} />, { project: true });
    fireEvent.click(screen.getByRole("button", { name: "Explain how this circuit works" }));
    expect(screen.getByLabelText("Message")).toHaveValue("Explain how this circuit works");
  });

  it("sends on Enter, keeps a newline on Shift+Enter, and streams the turn", async () => {
    const { store } = await renderWithStore(<ChatPanel width={400} />, { project: true });
    const box = screen.getByLabelText("Message");
    fireEvent.change(box, { target: { value: "Move the corner" } });
    fireEvent.keyDown(box, { key: "Enter", shiftKey: true });
    expect(store.get().chat.messages).toHaveLength(0);
    fireEvent.keyDown(box, { key: "Enter" });
    expect(box).toHaveValue("");
    await screen.findByRole("button", { name: /Edited rc_lowpass.asc/ });
    await screen.findByRole("button", { name: /Checked 3 specs/ });
    await waitFor(() => expect(store.get().chat.streaming).toBe(false));
    expect(screen.getByRole("button", { name: /Thinking/ })).toHaveAttribute("aria-expanded", "false");
  });

  it("shows an approval prompt in ask mode and applies on Apply", async () => {
    const { store } = await renderWithStore(<ChatPanel width={400} />, { project: true });
    await store.saveSettings({ ...store.get().settings!, edit_mode: "ask" });
    fireEvent.change(screen.getByLabelText("Message"), { target: { value: "Move the corner" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    const prompt = await screen.findByRole("alertdialog", { name: "Edit waiting for approval" });
    expect(prompt).toHaveTextContent("Apply this edit?");
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    await screen.findByRole("button", { name: /Edited rc_lowpass.asc/ });
  });

  it("shows a calm error with a retry when the turn fails", async () => {
    await renderWithStore(<ChatPanel width={400} />, { project: true });
    fireEvent.change(screen.getByLabelText("Message"), { target: { value: "Please fail with an error" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("529 Overloaded");
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });
});
