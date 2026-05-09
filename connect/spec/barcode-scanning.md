# Research: Barcode Scanning (Android)

> How to implement a camera-based barcode scanner in OHDC Android.

## Stack decision

The Android ecosystem offers a few options:
- **Google ML Kit barcode scanning** (recommended)
- **ZXing** (legacy, widely used, works without Google Play Services)
- **Scanbot SDK** (commercial, proprietary, overkill)
- **CameraX + ZXing** as a hand-rolled alternative

**We go with ML Kit + CameraX + Jetpack Compose.** Reasons:

1. **ML Kit's barcode scanner is fast, accurate, works offline**, and supports all major 1D and 2D formats (including EAN-13 used by most food products).
2. **CameraX** abstracts away the ugly Camera2 API lifecycle issues.
3. **Jetpack Compose** is our UI framework; Compose + CameraX + ML Kit integration is now well-documented and idiomatic.

There's one caveat: ML Kit depends on Google Play Services. For a personal-use app, that's fine. If we ever want an F-Droid-compatible variant, ZXing is the fallback.

## Supported barcode formats

ML Kit supports these formats out of the box:

- **1D**: Codabar, Code 39, Code 93, Code 128, EAN-8, EAN-13, ITF, UPC-A, UPC-E
- **2D**: Aztec, Data Matrix, PDF417, QR Code

For food products, **EAN-13 and UPC-A** are the primary formats. We can restrict scanning to these to improve speed and reduce false positives.

## Dependencies

```kotlin
// app/build.gradle.kts
dependencies {
    val cameraxVersion = "1.4.2"
    implementation("androidx.camera:camera-core:$cameraxVersion")
    implementation("androidx.camera:camera-camera2:$cameraxVersion")
    implementation("androidx.camera:camera-lifecycle:$cameraxVersion")
    implementation("androidx.camera:camera-view:$cameraxVersion")

    implementation("com.google.mlkit:barcode-scanning:17.3.0")

    // Compose (probably already present)
    implementation(platform("androidx.compose:compose-bom:2024.12.01"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")

    // Permissions helper
    implementation("com.google.accompanist:accompanist-permissions:0.37.0")
}
```

## Manifest

```xml
<uses-feature android:name="android.hardware.camera" android:required="false"/>
<uses-feature android:name="android.hardware.camera.autofocus" android:required="false"/>
<uses-permission android:name="android.permission.CAMERA"/>

<application ...>
    <!-- Tell Play Services to download the barcode model on install -->
    <meta-data
        android:name="com.google.mlkit.vision.DEPENDENCIES"
        android:value="barcode"/>
</application>
```

The `meta-data` hint tells Google Play Services to preload the barcode model on app install, reducing first-scan latency.

## Architecture

```
┌──────────────────────────────┐
│  BarcodeScannerScreen        │  Compose screen
│  (requests permissions,      │
│   renders preview + overlay) │
└──────┬──────────────┬────────┘
       │              │
       ▼              ▼
┌──────────────┐  ┌──────────────────┐
│ CameraX      │  │ BarcodeAnalyzer  │
│ PreviewView  │  │ (ImageAnalysis)  │
│ (Surface)    │  │                  │
└──────┬───────┘  └────────┬─────────┘
       │                   │
       └─────┬─────────────┘
             │
             ▼
    ┌─────────────────┐
    │ ML Kit Barcode  │
    │ Scanner         │
    └────────┬────────┘
             │
             │ onSuccess: barcode.rawValue
             ▼
    ┌─────────────────┐
    │ onBarcodeFound  │
    │ callback        │
    └────────┬────────┘
             │
             ▼
    OpenFoodFacts lookup → log form
```

## Reference implementation (Compose)

```kotlin
// BarcodeAnalyzer.kt
class BarcodeAnalyzer(
    private val onBarcode: (String) -> Unit,
) : ImageAnalysis.Analyzer {

    private val scanner: BarcodeScanner by lazy {
        val options = BarcodeScannerOptions.Builder()
            .setBarcodeFormats(
                Barcode.FORMAT_EAN_13,
                Barcode.FORMAT_EAN_8,
                Barcode.FORMAT_UPC_A,
                Barcode.FORMAT_UPC_E,
                Barcode.FORMAT_CODE_128, // some products
            )
            .build()
        BarcodeScanning.getClient(options)
    }

    @OptIn(ExperimentalGetImage::class)
    override fun analyze(imageProxy: ImageProxy) {
        val mediaImage = imageProxy.image
        if (mediaImage != null) {
            val image = InputImage.fromMediaImage(
                mediaImage,
                imageProxy.imageInfo.rotationDegrees
            )
            scanner.process(image)
                .addOnSuccessListener { barcodes ->
                    barcodes.firstOrNull()?.rawValue?.let(onBarcode)
                }
                .addOnCompleteListener {
                    imageProxy.close()
                }
        } else {
            imageProxy.close()
        }
    }
}
```

```kotlin
// BarcodeScannerScreen.kt
@OptIn(ExperimentalPermissionsApi::class)
@Composable
fun BarcodeScannerScreen(
    onBarcodeDetected: (String) -> Unit,
    onCancel: () -> Unit,
) {
    val cameraPermission = rememberPermissionState(Manifest.permission.CAMERA)

    LaunchedEffect(Unit) {
        if (!cameraPermission.status.isGranted) {
            cameraPermission.launchPermissionRequest()
        }
    }

    when {
        cameraPermission.status.isGranted -> {
            CameraPreview(
                onBarcodeDetected = onBarcodeDetected
            )
        }
        cameraPermission.status.shouldShowRationale -> {
            PermissionRationaleDialog(
                onRetry = { cameraPermission.launchPermissionRequest() },
                onCancel = onCancel,
            )
        }
        else -> {
            PermissionDeniedScreen(onCancel = onCancel)
        }
    }
}

@Composable
private fun CameraPreview(
    onBarcodeDetected: (String) -> Unit,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val executor = remember { Executors.newSingleThreadExecutor() }

    // Debounce: only fire the callback once per unique barcode scan
    var lastBarcode by remember { mutableStateOf<String?>(null) }

    DisposableEffect(Unit) {
        onDispose { executor.shutdown() }
    }

    Box(Modifier.fillMaxSize()) {
        AndroidView(
            modifier = Modifier.fillMaxSize(),
            factory = { ctx ->
                val previewView = PreviewView(ctx).apply {
                    implementationMode = PreviewView.ImplementationMode.COMPATIBLE
                    scaleType = PreviewView.ScaleType.FILL_CENTER
                }

                val cameraProviderFuture = ProcessCameraProvider.getInstance(ctx)
                cameraProviderFuture.addListener({
                    val cameraProvider = cameraProviderFuture.get()
                    val preview = Preview.Builder().build().apply {
                        surfaceProvider = previewView.surfaceProvider
                    }
                    val analyzer = ImageAnalysis.Builder()
                        .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                        .build().apply {
                            setAnalyzer(executor, BarcodeAnalyzer { barcode ->
                                if (barcode != lastBarcode) {
                                    lastBarcode = barcode
                                    onBarcodeDetected(barcode)
                                }
                            })
                        }
                    val selector = CameraSelector.DEFAULT_BACK_CAMERA
                    cameraProvider.unbindAll()
                    cameraProvider.bindToLifecycle(
                        lifecycleOwner, selector, preview, analyzer
                    )
                }, ContextCompat.getMainExecutor(ctx))

                previewView
            }
        )

        // Overlay: viewfinder rectangle, instructions, cancel button
        ScannerOverlay()
    }
}
```

## UX considerations

### Viewfinder overlay

Don't just show the full camera feed — overlay a "scan here" rectangle to tell the user where to aim. Successful scans inside the rectangle should trigger a brief visual confirmation (flash, rectangle color change) and haptic feedback.

### Debouncing

ML Kit will detect the same barcode on consecutive frames. Without debouncing, the user triggers the "found!" callback 60 times per second. Keep state of the last detected barcode; only fire the callback once per *new* barcode detected. Reset after the user cancels or navigates away.

### Torch / flashlight

In low-light conditions (kitchen lighting isn't great), provide a torch toggle button. CameraX supports this via `camera.cameraControl.enableTorch(true)`.

### Fallback: manual entry

If the user can't scan (barcode worn, product doesn't have one, wrong lighting), provide an easy "enter barcode manually" button or a "search by name" fallback.

### Permission denied handling

If the user denies camera permission, show a clear explanation and a button to open app settings. Don't dead-end the user.

## Performance notes

- **Use `STRATEGY_KEEP_ONLY_LATEST`** on `ImageAnalysis`. Otherwise the scanner can get backlogged on slow devices.
- **Restrict barcode formats** to the ones you care about (EAN-13, UPC-A) — faster than scanning all formats.
- **Use a dedicated single-thread executor** for the analyzer; don't run it on the main thread.
- **First scan after app install can be slow** (5–10 seconds) because Play Services downloads the barcode model. The `meta-data` hint in the manifest triggers this download at install time instead.

## Privacy considerations

The camera feed is processed entirely on-device by ML Kit; no frames are sent to Google. This is a genuine privacy win — we can truthfully say the camera feed never leaves the device.

However: once we have the barcode, the next step is the OpenFoodFacts API call, which does leave the device. The user should understand the flow.

## iOS equivalent (future)

On iOS, the equivalent stack is:
- `AVFoundation` for camera.
- `Vision` framework's `VNDetectBarcodesRequest` for detection (on-device, fast, built in).
- SwiftUI for the UI.

Similar architecture, similar UX. Not Phase 1.

## Open questions

- **Quantity estimation from a scan.** Can we use the camera feed to estimate the weight of a banana or the volume of a drink? Maybe with a reference object (scale the app knows the size of, like a credit card). Far future.
- **Scanning multiple items at once (shopping cart mode).** Useful for meal prep logging. Phase 2+.
- **Barcodes that aren't food.** User scans a pharmacy barcode — need to handle that gracefully. Probably: if it's not in OpenFoodFacts, try the drug database (next research doc).
