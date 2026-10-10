import type { ProviderId, ProviderStatus, SimulatorId } from "../ipc/types";

export const PROVIDER_NAMES: Record<ProviderId, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  google: "Google",
  openrouter: "OpenRouter",
  ollama: "Ollama",
  custom: "Custom endpoint",
};

export const PROVIDER_ORDER: ProviderId[] = ["anthropic", "openai", "google", "openrouter", "ollama", "custom"];

export const DEFAULT_BASE_URLS: Record<ProviderId, string> = {
  anthropic: "https://api.anthropic.com",
  openai: "https://api.openai.com/v1",
  google: "https://generativelanguage.googleapis.com/v1beta/openai",
  openrouter: "https://openrouter.ai/api/v1",
  ollama: "http://localhost:11434/v1",
  custom: "https://your-server/v1",
};

export const SIMULATOR_NAMES: Record<SimulatorId, string> = {
  ngspice: "ngspice",
  ltspice: "LTspice",
  xyce: "Xyce",
  spectre: "Spectre",
};

export const SIMULATOR_ORDER: SimulatorId[] = ["ngspice", "ltspice", "xyce", "spectre"];

/** What to do when a simulator is missing, if the backend gave no note. */
export const SIMULATOR_HINTS: Record<SimulatorId, string> = {
  ngspice: "Install ngspice (apt install ngspice, brew install ngspice, or the Windows build from ngspice.sourceforge.io).",
  ltspice: "Install LTspice, or set its path in Settings.",
  xyce: "Optional. Install Xyce for large transient runs.",
  spectre: "Optional. Needs an SSH host in the config file.",
};

export function keySourceText(p: ProviderStatus): string {
  if (p.id === "ollama" && p.source === null) return p.configured ? "No key needed" : "Not running";
  if (!p.configured) return "No key";
  if (p.source === "keychain") return "Key in system keychain";
  if (p.source === "file") return "Key in config file";
  if (p.source === "env") return "Key from environment";
  return "Configured";
}

export function providerHint(p: ProviderStatus): string | null {
  if (p.configured) return null;
  if (p.id === "ollama") return "Start Ollama, then check again.";
  if (p.id === "custom") return "Set a base URL and key in Settings.";
  return "Add a key in Settings.";
}
