package org.droidwin.blank;

import android.app.Activity;
import android.graphics.Color;
import android.os.Bundle;
import android.util.Log;
import android.view.View;

/**
 * An app whose launcher activity deliberately draws one solid colour.
 *
 * It exists so DroidWin's diagnosis can be tested against a real blank window:
 * a live process, a real Android display, and nothing on it. The log line below
 * is what the evidence panel quotes, so the blank render is confirmed from a
 * screenshot *and* the app's own log, not inferred.
 */
public class BlankActivity extends Activity {
    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        View surface = new View(this);
        surface.setBackgroundColor(Color.BLACK);
        setContentView(surface);
        Log.e("org.droidwin.blank", "blank surface drawn; no content view");
    }
}
