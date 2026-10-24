package ai.storyteller.figcraft;

import android.app.Activity;
import android.content.Intent;
import android.database.Cursor;
import android.provider.OpenableColumns;
import android.net.Uri;
import android.provider.DocumentsContract;
import android.util.Log;

import androidx.annotation.NonNull;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;

/**
 * The Android entry point, and the Storage Access Framework side of the file pickers.
 *
 * <p>{@code android-activity} launches this class because the manifest names it and the native
 * library is loaded through {@code android.app.lib_name=main}. Everything that needs a Java API and
 * would otherwise force JNI marshalling of file contents lives here; the Rust side (src/saf.rs)
 * only ever calls these methods and then reads {@code <filesDir>/import-manifest.txt}.
 *
 * <p>The division of labour is deliberate: a picked document is a {@code content://} URI, and the
 * engines behind the Craft apps work in paths. So {@link #safPickMedia()} copies each picked
 * document into {@code filesDir/import/} and writes the resulting real paths to the manifest. The
 * Rust side polls {@link #safIsDone()} on a background thread and reads that file, which keeps the
 * JNI surface at six method calls and never blocks the render thread.
 */
public class MainActivity extends com.google.androidgamesdk.GameActivity {
    private static final String TAG = "FigcraftSAF";
    private static final int REQ_OPEN = 0x5AF1;
    private static final int REQ_SAVE = 0x5AF2;
    private static final String EXTRA_SAVE_NAME = "saveName";

    /** One picked path per line; the Rust side reads this and deletes it on the next pick. */
    private static final String MANIFEST = "import-manifest.txt";

    private volatile boolean importDone = true;
    private volatile boolean saveDone = true;
    private volatile boolean hasSaveUri = false;
    private Uri saveUri = null;

    /** ACTION_OPEN_DOCUMENT, any number of files. Results land in the manifest. */
    public void safPickMedia() {
        importDone = false;
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
        /* Android-port: vivo's file manager decides which UI to show from the MIME type. With a
         * bare all-types MIME it opens its *media* selector (图片 / 时光轴 / 相册集), where a .svg,
         * .pdf or .blend cannot be selected at all -- the picker "appears" but is useless for
         * documents. A non-visual EXTRA_MIME_TYPES list makes it open the real document browser
         * (全部 / 图片 / 文档 / 其他, with 最近 and 设备存储). Verified on the device. */
        intent.putExtra(Intent.EXTRA_MIME_TYPES, new String[] {
            "application/octet-stream",
            "application/pdf",
            "image/svg+xml",
            "text/plain",
        });
        runOnUiThread(() -> {
            try {
                startActivityForResult(intent, REQ_OPEN);
            } catch (Exception e) {
                Log.w(TAG, "no activity can handle ACTION_OPEN_DOCUMENT", e);
                importDone = true;
            }
        });
    }

    /** True once the import picker has finished and the manifest is written. */
    public boolean safIsDone() {
        return importDone;
    }

    /** ACTION_CREATE_DOCUMENT; the chosen URI is kept here until safPublish spends it. */
    public void safPickSave(String suggestedName) {
        saveDone = false;
        hasSaveUri = false;
        saveUri = null;
        Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        intent.putExtra(Intent.EXTRA_TITLE, suggestedName);
        runOnUiThread(() -> {
            try {
                startActivityForResult(intent, REQ_SAVE);
            } catch (Exception e) {
                Log.w(TAG, "no activity can handle ACTION_CREATE_DOCUMENT", e);
                saveDone = true;
            }
        });
    }

    /** True once the save picker has finished (whether or not a destination was chosen). */
    public boolean safSaveDone() {
        return saveDone;
    }

    /** True when the user actually chose a destination, i.e. safPublish has somewhere to write. */
    public boolean safHasSaveUri() {
        return hasSaveUri;
    }

    /** Copy a finished file to the destination the save picker returned. */
    public boolean safPublish(String path) {
        Uri destination = saveUri;
        if (destination == null) {
            Log.w(TAG, "safPublish with no destination");
            return false;
        }
        File source = new File(path);
        if (!source.isFile()) {
            Log.w(TAG, "safPublish: not a file: " + path);
            return false;
        }
        try (InputStream in = new java.io.FileInputStream(source);
             OutputStream out = getContentResolver().openOutputStream(destination, "wt")) {
            if (out == null) {
                Log.w(TAG, "safPublish: openOutputStream returned null");
                return false;
            }
            byte[] buffer = new byte[64 * 1024];
            int read;
            long total = 0;
            while ((read = in.read(buffer)) > 0) {
                out.write(buffer, 0, read);
                total += read;
            }
            out.flush();
            Log.i(TAG, "published " + total + " bytes to " + destination);
            return true;
        } catch (IOException e) {
            Log.w(TAG, "safPublish failed", e);
            return false;
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == REQ_OPEN) {
            try {
                writeImportManifest(resultCode == Activity.RESULT_OK ? data : null);
            } catch (Throwable t) {
                Log.w(TAG, "import failed", t);
            } finally {
                importDone = true;
            }
        } else if (requestCode == REQ_SAVE) {
            try {
                if (resultCode == Activity.RESULT_OK && data != null && data.getData() != null) {
                    saveUri = data.getData();
                    // Without this the grant dies with the process, so a later publish would fail.
                    try {
                        getContentResolver().takePersistableUriPermission(
                                saveUri, Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
                    } catch (SecurityException ignored) {
                        // Not every provider offers a persistable grant; the one-shot grant is enough.
                    }
                    hasSaveUri = true;
                    Log.i(TAG, "save destination " + saveUri);
                } else {
                    Log.i(TAG, "save cancelled");
                }
            } finally {
                saveDone = true;
            }
        }
    }

    /**
     * Copy whatever the import picker returned into filesDir/import and record the paths.
     *
     * <p>Each document gets its display name; a collision gets a numeric suffix rather than
     * overwriting, because the user may well pick two files with the same name from different
     * folders and losing one silently would be worse than an odd filename.
     */
    private void writeImportManifest(Intent data) throws IOException {
        File manifest = new File(getFilesDir(), MANIFEST);
        if (data == null) {
            // Write an empty manifest rather than leaving a stale one behind.
            try (FileOutputStream out = new FileOutputStream(manifest, false)) {
                // nothing
            }
            Log.i(TAG, "import cancelled; manifest cleared");
            return;
        }

        ArrayList<Uri> uris = new ArrayList<>();
        android.content.ClipData clip = data.getClipData();
        if (clip != null) {
            for (int i = 0; i < clip.getItemCount(); i++) {
                Uri uri = clip.getItemAt(i).getUri();
                if (uri != null) {
                    uris.add(uri);
                }
            }
        } else if (data.getData() != null) {
            uris.add(data.getData());
        }

        File dir = new File(getFilesDir(), "import");
        if (!dir.isDirectory() && !dir.mkdirs()) {
            Log.w(TAG, "could not create " + dir);
            return;
        }

        StringBuilder lines = new StringBuilder();
        for (Uri uri : uris) {
            try {
                getContentResolver().takePersistableUriPermission(
                        uri, Intent.FLAG_GRANT_READ_URI_PERMISSION);
            } catch (SecurityException ignored) {
                // Fine: the copy below happens while the one-shot grant is still live.
            }
            String name = displayName(uri);
            File target = uniqueTarget(dir, name);
            try (InputStream in = getContentResolver().openInputStream(uri);
                 OutputStream out = new FileOutputStream(target)) {
                if (in == null) {
                    Log.w(TAG, "openInputStream returned null for " + uri);
                    continue;
                }
                byte[] buffer = new byte[64 * 1024];
                int read;
                while ((read = in.read(buffer)) > 0) {
                    out.write(buffer, 0, read);
                }
            } catch (IOException e) {
                Log.w(TAG, "could not copy " + uri, e);
                continue;
            }
            lines.append(target.getAbsolutePath()).append('\n');
            Log.i(TAG, "imported " + uri + " -> " + target);
        }
        try (FileOutputStream out = new FileOutputStream(manifest, false)) {
            out.write(lines.toString().getBytes("UTF-8"));
        }
        Log.i(TAG, "manifest has " + lines.toString().split("\n").length + " path(s)");
    }

    private static File uniqueTarget(File dir, String name) {
        File target = new File(dir, name);
        if (!target.exists()) {
            return target;
        }
        int dot = name.lastIndexOf('.');
        String stem = dot > 0 ? name.substring(0, dot) : name;
        String suffix = dot > 0 ? name.substring(dot) : "";
        for (int n = 2; ; n++) {
            target = new File(dir, stem + " (" + n + ")" + suffix);
            if (!target.exists()) {
                return target;
            }
        }
    }

    /** The document's display name, falling back to the last path segment. */
    private String displayName(@NonNull Uri uri) {
        String name = null;
        // Android-port: DISPLAY_NAME is the only place the *extension* survives. The document id
        // below is an opaque number for media-provider URIs, so naming the copy after it produces
        // an extension-less file that the app cannot identify. Ask the provider first.
        try (Cursor c = getContentResolver().query(uri,
                new String[] { OpenableColumns.DISPLAY_NAME }, null, null, null)) {
            if (c != null && c.moveToFirst() && !c.isNull(0)) {
                name = c.getString(0);
            }
        } catch (Throwable ignored) {
            // Provider does not implement DISPLAY_NAME; fall back to the document id.
        }
        try {
            if (name == null || name.isEmpty()) {
                name = DocumentsContract.getDocumentId(uri);
                int slash = name.lastIndexOf('/');
                if (slash >= 0) {
                    name = name.substring(slash + 1);
                }
                int colon = name.indexOf(':');
                if (colon >= 0) {
                    name = name.substring(colon + 1);
                }
            }
        } catch (Throwable ignored) {
            // Not a document URI after all; use the path segment below.
        }
        if (name == null || name.isEmpty()) {
            name = uri.getLastPathSegment();
        }
        if (name == null || name.isEmpty()) {
            name = "imported";
        }
        return name.replace('/', '_');
    }
}
