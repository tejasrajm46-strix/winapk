import { IconFolder, IconInstall } from "./icons";
import { Button } from "./ui";

export function DropZone({
  onBrowse,
  active,
  busy,
  hint,
}: {
  onBrowse: () => void;
  active?: boolean;
  busy?: boolean;
  hint?: string;
}) {
  return (
    <div className={`dropzone${active ? " dropzone--active" : ""}`}>
      <IconInstall size={22} />
      <p className="dropzone__title">Drop an APK here</p>
      <p className="dropzone__hint">
        {hint ??
          "WINAPK reads the package without executing it. You review the manifest before anything is installed."}
      </p>
      <div className="dropzone__actions">
        <Button variant="primary" onClick={onBrowse} disabled={busy}>
          <IconFolder size={14} />
          Browse APK
        </Button>
      </div>
    </div>
  );
}
