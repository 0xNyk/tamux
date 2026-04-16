import { useEffect, useState } from "react";
import { getBridge } from "@/lib/bridge";
import { isCDUIEnabled, setCDUIEnabled } from "../../lib/cduiMode";
import { useAgentStore } from "../../lib/agentStore";
import { Section, SettingRow, Toggle, smallBtnStyle } from "./shared";

function defaultViewsPathLabel(): string {
    if (typeof navigator !== "undefined") {
        const ua = navigator.userAgent.toLowerCase();
        if (ua.includes("windows")) {
            return "%LOCALAPPDATA%\\tamux\\views";
        }
        if (ua.includes("mac")) {
            return "~/Library/Application Support/tamux/views";
        }
    }
    return "~/.tamux/views";
}

export function AboutTab() {
    const [cduiEnabled, setCduiEnabledState] = useState<boolean>(() => isCDUIEnabled());
    const [viewsPathLabel, setViewsPathLabel] = useState<string>(() => defaultViewsPathLabel());
    const operatorProfile = useAgentStore((s) => s.operatorProfile);
    const getOperatorProfileSummary = useAgentStore((s) => s.getOperatorProfileSummary);
    const setOperatorProfileConsent = useAgentStore((s) => s.setOperatorProfileConsent);
    const startOperatorProfileSession = useAgentStore((s) => s.startOperatorProfileSession);
    const setOperatorProfilePanelOpen = useAgentStore((s) => s.setOperatorProfilePanelOpen);

    const summary = operatorProfile.summary;
    const profileFields = summary?.fields ?? {};
    const consentMap = Object.fromEntries((summary?.consents ?? []).map((entry) => [entry.consent_key, entry]));

    const checkins = summary?.checkins ?? [];
    const sortedCheckins = [...checkins].sort((a, b) => (b.scheduled_at ?? 0) - (a.scheduled_at ?? 0));
    const nextScheduledCheckin = checkins
        .filter((entry) => typeof entry.scheduled_at === "number" && entry.scheduled_at > Date.now())
        .sort((a, b) => (a.scheduled_at ?? 0) - (b.scheduled_at ?? 0))[0] ?? null;
    const lastCheckin = sortedCheckins[0] ?? null;

    useEffect(() => {
        const bridge = getBridge();
        if (!bridge?.getDataDir) return;
        bridge.getDataDir()
            .then((dataDir: string) => {
                if (typeof dataDir !== "string" || !dataDir.trim()) return;
                const separator = dataDir.includes("\\") ? "\\" : "/";
                setViewsPathLabel(`${dataDir}${separator}views`);
            })
            .catch(() => { });
    }, []);

    useEffect(() => {
        void getOperatorProfileSummary();
    }, [getOperatorProfileSummary]);

    const getConsentValue = (key: string, defaultValue = true): boolean =>
        consentMap[key]?.granted ?? defaultValue;

    const formatDateTime = (unixMs: number | null | undefined): string => {
        if (typeof unixMs !== "number" || !Number.isFinite(unixMs) || unixMs <= 0) {
            return "Not scheduled";
        }
        try {
            return new Date(unixMs).toLocaleString();
        } catch {
            return "Not scheduled";
        }
    };

    const renderProfileValue = (key: string): string => {
        const entry = profileFields[key];
        if (!entry) {
            return "Not set";
        }
        if (typeof entry.value === "string") {
            return entry.value;
        }
        try {
            return JSON.stringify(entry.value);
        } catch {
            return "Not set";
        }
    };

    const applyRuntimeMode = () => {
        setCDUIEnabled(cduiEnabled);
        window.location.reload();
    };

    return (
        <>
            <Section title="Runtime Mode">
                <SettingRow label="Use New CDUI (this will reload the app immediately!)">
                    <Toggle value={cduiEnabled} onChange={() => {
                        setCduiEnabledState((prev) => !prev);
                        setCDUIEnabled(!cduiEnabled);
                    }} />
                </SettingRow>
                <div style={{ marginTop: 8, display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12 }}>
                    <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>
                        CDUI is now the default interface. Turn it off here to reload into the legacy UI, or turn it back on to use YAML-driven views from {viewsPathLabel}.
                    </span>
                    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                        <button
                            onClick={() => {
                                void (getBridge())?.revealDataPath?.("views");
                            }}
                            style={smallBtnStyle}
                        >
                            Open {viewsPathLabel}
                        </button>
                        <button
                            onClick={() => {
                                window.dispatchEvent(new Event("tamux-cdui-views-reload"));
                                window.dispatchEvent(new Event("amux-cdui-views-reload"));
                            }}
                            style={smallBtnStyle}
                        >
                            Reload Views
                        </button>
                        <button onClick={applyRuntimeMode} style={smallBtnStyle}>
                            Apply & Reload
                        </button>
                        <button
                            onClick={() => {
                                window.dispatchEvent(new Event("tamux-open-setup-onboarding"));
                                window.dispatchEvent(new Event("amux-open-setup-onboarding"));
                            }}
                            style={smallBtnStyle}
                        >
                            Open Setup Assistant
                        </button>
                    </div>
                </div>
            </Section>

            <Section title="About">
                <div style={{ fontSize: 13, lineHeight: 1.6 }}>
                    <p style={{ fontWeight: 600, marginBottom: 8 }}>tamux - Terminal Multiplexer</p>
                    <p>Version 0.5.3</p>
                    <p style={{ marginTop: 8, color: "var(--text-secondary)" }}>
                        A cross-platform terminal multiplexer with workspaces, surfaces, pane management,
                        AI agent integration, snippet library, and session persistence.
                    </p>
                    <p style={{ marginTop: 12, color: "var(--text-secondary)" }}>
                        Built with Electron, React, xterm.js, and a Rust daemon.
                    </p>
                </div>
            </Section>

            <Section title="About You">
                <div style={{ display: "grid", gap: 10 }}>
                    <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>
                        Personalization profile used for onboarding, check-ins, and proactive suggestions.
                    </div>

                    <div style={{
                        border: "1px solid rgba(255,255,255,0.08)",
                        background: "rgba(255,255,255,0.02)",
                        padding: 10,
                        display: "grid",
                        gap: 6,
                    }}>
                        <div style={{ fontWeight: 700, fontSize: 13 }}>Profile Summary</div>
                        <div style={{ fontSize: 12 }}>
                            <strong>Name:</strong> {renderProfileValue("name")}
                        </div>
                        <div style={{ fontSize: 12 }}>
                            <strong>Role:</strong> {renderProfileValue("role")}
                        </div>
                        <div style={{ fontSize: 12 }}>
                            <strong>Primary language:</strong> {renderProfileValue("primary_language")}
                        </div>
                        <div style={{ fontSize: 12 }}>
                            <strong>Work style:</strong> {renderProfileValue("work_style")}
                        </div>
                    </div>

                    <div style={{
                        border: "1px solid rgba(255,255,255,0.08)",
                        background: "rgba(255,255,255,0.02)",
                        padding: 10,
                        display: "grid",
                        gap: 6,
                    }}>
                        <div style={{ fontWeight: 700, fontSize: 13 }}>Consent Controls</div>
                        <SettingRow label="Passive learning">
                            <Toggle
                                value={getConsentValue("passive_learning", true)}
                                onChange={(next) => { void setOperatorProfileConsent("passive_learning", next); }}
                            />
                        </SettingRow>
                        <SettingRow label="Weekly check-ins">
                            <Toggle
                                value={getConsentValue("weekly_checkins", true)}
                                onChange={(next) => { void setOperatorProfileConsent("weekly_checkins", next); }}
                            />
                        </SettingRow>
                        <SettingRow label="Proactive suggestions">
                            <Toggle
                                value={getConsentValue("proactive_suggestions", true)}
                                onChange={(next) => { void setOperatorProfileConsent("proactive_suggestions", next); }}
                            />
                        </SettingRow>
                    </div>

                    <div style={{
                        border: "1px solid rgba(255,255,255,0.08)",
                        background: "rgba(255,255,255,0.02)",
                        padding: 10,
                        display: "grid",
                        gap: 6,
                    }}>
                        <div style={{ fontWeight: 700, fontSize: 13 }}>Next Check-in</div>
                        <div style={{ fontSize: 12 }}>
                            <strong>Next scheduled:</strong> {formatDateTime(nextScheduledCheckin?.scheduled_at)}
                        </div>
                        <div style={{ fontSize: 12 }}>
                            <strong>Last check-in:</strong>{" "}
                            {lastCheckin
                                ? `${lastCheckin.kind} (${lastCheckin.status}) at ${formatDateTime(lastCheckin.shown_at ?? lastCheckin.scheduled_at)}`
                                : "No check-ins recorded"}
                        </div>
                    </div>

                    <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                        <button
                            type="button"
                            onClick={() => { void getOperatorProfileSummary(); }}
                            style={smallBtnStyle}
                        >
                            Refresh Profile
                        </button>
                        <button
                            type="button"
                            onClick={() => {
                                setOperatorProfilePanelOpen(true);
                                void startOperatorProfileSession("refresh");
                            }}
                            style={smallBtnStyle}
                        >
                            Update About You
                        </button>
                    </div>
                </div>
            </Section>
        </>
    );
}
