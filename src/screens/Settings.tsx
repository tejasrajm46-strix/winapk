import { IconAlert, IconFolder } from "../components/icons";
import { Button, Field, Panel } from "../components/ui";
import { PageHead, type ScreenProps } from "./common";

export function Settings(props: ScreenProps) {
  const { env, status } = props;

  return (
    <div className="page">
      <PageHead
        eyebrow="Management"
        title="Settings"
        lede="DroidWin shows a switch only when it changes something real, so this page is mostly facts."
      >
        <Button variant="secondary" onClick={props.onOpenLogs}>
          <IconFolder size={14} />
          Open log folder
        </Button>
      </PageHead>

      <div className="grid grid--2">
        <Panel title="Where things live" flush>
          <dl className="fields" style={{ padding: "0 var(--space-lg) var(--space-md)" }}>
            <Field label="Android SDK" value={<span className="mono">{env?.sdk_root ?? "not found"}</span>} missing={!env?.sdk_root} />
            <Field label="System image" value={<span className="mono">{env?.system_image ?? "not installed"}</span>} missing={!env?.system_image} />
            <Field label="Virtual device" value={<span className="mono">{status?.avd ?? "not created"}</span>} missing={!status?.avd} />
            <Field label="Window host" value={<span className="mono">{env?.scrcpy ?? "missing"}</span>} missing={!env?.scrcpy} />
            <Field label="Package inspector" value={<span className="mono">{env?.aapt2 ?? "missing (labels stay unresolved)"}</span>} missing={!env?.aapt2} />
          </dl>
        </Panel>

        <Panel title="About this build">
          <dl className="fields">
            <Field label="Version" value="0.1.0 (development build)" />
            <Field label="Runtime model" value="One shared Android emulator, one virtual display per app window" />
            <Field label="Licensing" value="scrcpy is bundled under Apache-2.0. The Android SDK and emulator are provided by you and are not redistributed by DroidWin." />
            <Field label="Fonts" value="Instrument Serif and Onest, bundled under the SIL Open Font License 1.1" />
          </dl>
        </Panel>
      </div>

      <Panel title="Not implemented yet" note="Listed so nothing here looks like a missing switch.">
        <ul className="stack stack--sm secondary" style={{ paddingLeft: 18, margin: 0 }}>
          <li>Deleting an app's Android data from the library (the backend supports it; the control is not built).</li>
          <li>Automatic SDK provisioning — DroidWin tells you what to install instead of downloading Google's SDK.</li>
          <li>Audio forwarding, which is why windows open with audio disabled.</li>
          <li>.apk file association, which ships with the installer in a later build.</li>
          <li>Cloud sync, accounts, or any telemetry.</li>
        </ul>
        <div className="note note--warn">
          <div className="note__head">
            <IconAlert size={14} />
            <span>Compatibility is not universal</span>
          </div>
          <p className="note__body">
            Apps are only supported if their CPU ABI, Android API level, graphics use and services
            requirements match the runtime. ARM-only packages cannot run on this x86_64 image, and
            apps that insist on Google Play services will fail. DroidWin reports what Android reports
            rather than predicting compatibility.
          </p>
        </div>
      </Panel>
    </div>
  );
}
