import type { Doctor } from "../ipc/types";
import { MOD } from "../lib/keys";
import { keySourceText, PROVIDER_NAMES, PROVIDER_ORDER, providerHint, SIMULATOR_HINTS, SIMULATOR_NAMES } from "../lib/names";
import { useStore, useStoreApi } from "../store/context";
import { Mark } from "./Mark";
import "./Welcome.css";

export function Welcome() {
  const store = useStoreApi();
  const recent = useStore((s) => s.recent);
  const opening = useStore((s) => s.opening);
  const doctor = useStore((s) => s.doctor);
  const doctorError = useStore((s) => s.doctorError);

  return (
    <main className="welcome">
      <div className="welcome-inner">
        <header className="welcome-head">
          <Mark size={44} />
          <div>
            <h1 className="welcome-title">aispice</h1>
            <p className="welcome-lede">An agent that reads, edits and simulates your SPICE schematics.</p>
          </div>
        </header>

        <div className="welcome-actions">
          <button type="button" className="button button-primary button-tall" onClick={() => void store.openFolder()} disabled={opening}>
            {opening ? "Opening..." : "Open folder"}
            <kbd className="welcome-kbd">{MOD}+O</kbd>
          </button>
          <button type="button" className="button button-tall" onClick={() => store.openSettings()}>
            Open settings
          </button>
        </div>

        <div className="welcome-grid">
          <section className="panel" aria-labelledby="welcome-recent">
            <h2 className="panel-title" id="welcome-recent">
              Recent projects
            </h2>
            {recent.length === 0 ? (
              <p className="panel-note">No recent projects. Open a folder that holds .asc files.</p>
            ) : (
              <ul className="recent-list">
                {recent.map((path) => (
                  <li key={path}>
                    <button type="button" className="recent-item" onClick={() => void store.openProject(path)} disabled={opening}>
                      <span className="recent-name">{path.split(/[\\/]/).pop()}</span>
                      <span className="recent-path mono">{path}</span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>

          <section className="panel" aria-labelledby="welcome-env">
            <div className="env-head">
              <h2 className="panel-title" id="welcome-env">
                Environment
              </h2>
              <button type="button" className="button button-quiet" onClick={() => void store.refreshDoctor()}>
                Check again
              </button>
            </div>
            {doctorError ? (
              <p className="panel-note error-text">Could not check the environment: {doctorError}</p>
            ) : doctor === null ? (
              <p className="panel-note">Checking simulators and providers...</p>
            ) : (
              <Environment doctor={doctor} />
            )}
          </section>
        </div>
      </div>
    </main>
  );
}

function Environment({ doctor }: { doctor: Doctor }) {
  const providers = PROVIDER_ORDER.map((id) => doctor.providers.find((p) => p.id === id)).filter((p) => p !== undefined);
  return (
    <div className="env">
      <h3 className="section-title">Simulators</h3>
      <ul className="env-list">
        {doctor.simulators.map((s) => (
          <li key={s.id} className="env-row">
            <span className="dot" data-on={s.found ? "" : undefined} aria-hidden="true" />
            <div className="env-main">
              <p className="env-line">
                <span className="env-name">{SIMULATOR_NAMES[s.id]}</span>
                <span className="env-state">{s.found ? (s.version ?? "found") : "Not found"}</span>
              </p>
              {s.found && s.path ? <p className="env-sub mono" title={s.path}>{s.path}</p> : null}
              {!s.found ? <p className="env-hint">{s.notes[0] ?? SIMULATOR_HINTS[s.id]}</p> : null}
              {s.found && s.notes.length > 0 ? <p className="env-sub">{s.notes.join(" ")}</p> : null}
            </div>
          </li>
        ))}
      </ul>

      <h3 className="section-title">LTspice library</h3>
      <div className="env-row">
        <span className="dot" data-on={doctor.ltspice_lib ? "" : undefined} aria-hidden="true" />
        <div className="env-main">
          {doctor.ltspice_lib ? (
            <p className="env-sub mono" title={doctor.ltspice_lib}>
              {doctor.ltspice_lib}
            </p>
          ) : (
            <>
              <p className="env-line">
                <span className="env-state">Not found</span>
              </p>
              <p className="env-hint">Parts use the built-in symbol set. Set the LTspice path in Settings to use its library.</p>
            </>
          )}
        </div>
      </div>

      <h3 className="section-title">Model providers</h3>
      <ul className="env-list">
        {providers.map((p) => {
          const hint = providerHint(p);
          return (
            <li key={p.id} className="env-row">
              <span className="dot" data-on={p.configured ? "" : undefined} aria-hidden="true" />
              <div className="env-main">
                <p className="env-line">
                  <span className="env-name">{PROVIDER_NAMES[p.id]}</span>
                  <span className="env-state">{keySourceText(p)}</span>
                </p>
                {hint ? <p className="env-hint">{hint}</p> : null}
              </div>
            </li>
          );
        })}
      </ul>

      <p className="env-foot mono">
        aispice {doctor.version} on {doctor.platform}
      </p>
    </div>
  );
}
