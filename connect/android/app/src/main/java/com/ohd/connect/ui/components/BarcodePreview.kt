package com.ohd.connect.ui.components

import android.Manifest
import android.content.pm.PackageManager
import android.graphics.ColorMatrix
import android.graphics.ColorMatrixColorFilter
import android.graphics.RenderEffect
import android.os.Build
import android.util.Log
import android.util.Size
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import java.util.concurrent.Executors

private const val TAG = "OhdBarcodePreview"

/**
 * Live camera preview with embedded ML Kit barcode detection.
 *
 * Drops into the 207 dp scan-area frame of `FoodScreen` / `FoodSearchScreen`.
 * Lifecycle-aware: the camera binds on entry and is released on dispose.
 *
 * Usage:
 *  - First entry → asks for `CAMERA` permission. If denied, renders a
 *    fallback prompt with a "Grant camera access" button that re-requests.
 *  - When permission is granted → live preview starts; ML Kit scans every
 *    frame for EAN/UPC/Code-128/QR. The first valid read fires [onScanned]
 *    once (state-debounced via `delivered`).
 *  - The composable doesn't navigate or popBackStack itself; the caller
 *    decides what to do with the scanned value.
 *
 * The preview uses `PreviewView` (an Android `View`) wrapped in
 * `AndroidView` because Compose doesn't natively render `SurfaceView` /
 * `TextureView`-style content yet.
 */
@Composable
fun BarcodePreview(
    onScanned: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val ctx = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current

    var hasPermission by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(ctx, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED,
        )
    }

    // Ask once on first composition if we don't already have permission.
    val permissionLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.RequestPermission(),
    ) { granted -> hasPermission = granted }

    // Auto-prompt on entry — matches what the user expects when they tap
    // into Food.
    DisposableEffect(Unit) {
        if (!hasPermission) {
            permissionLauncher.launch(Manifest.permission.CAMERA)
        }
        onDispose { /* nothing — launcher cleanup is automatic */ }
    }

    Box(modifier = modifier) {
        if (hasPermission) {
            CameraXBarcodePreview(
                onScanned = onScanned,
                modifier = Modifier.fillMaxSize(),
            )

            // Centred 90%-wide red "laser line" overlay so the user knows
            // where to align the barcode. `fillMaxWidth(0.9f)` keeps the
            // line at 90% width; `height(1.5.dp)` gives a hairline that
            // reads cleanly on xxhdpi without looking heavy.
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth(0.9f)
                        .height(1.5.dp)
                        .background(OhdColors.Red.copy(alpha = 0.85f)),
                )
            }

            Text(
                text = "Point camera at barcode",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = Color.White.copy(alpha = 0.7f),
                modifier = Modifier
                    .align(Alignment.BottomEnd)
                    .padding(12.dp),
            )
        } else {
            CameraPermissionFallback(
                onRequest = { permissionLauncher.launch(Manifest.permission.CAMERA) },
                modifier = Modifier.fillMaxSize(),
            )
        }
    }
}

@Composable
private fun CameraXBarcodePreview(
    onScanned: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val ctx = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val onScannedState = rememberUpdatedState(onScanned)

    // Single-shot debounce so we don't fire `onScanned` for every frame
    // showing the same barcode.
    val delivered = remember { java.util.concurrent.atomic.AtomicBoolean(false) }

    val analysisExecutor = remember { Executors.newSingleThreadExecutor() }
    val barcodeScanner = remember {
        BarcodeScanning.getClient(
            BarcodeScannerOptions.Builder()
                .setBarcodeFormats(
                    Barcode.FORMAT_EAN_13,
                    Barcode.FORMAT_EAN_8,
                    Barcode.FORMAT_UPC_A,
                    Barcode.FORMAT_UPC_E,
                    Barcode.FORMAT_CODE_128,
                    Barcode.FORMAT_QR_CODE,
                )
                .build(),
        )
    }
    DisposableEffect(Unit) {
        onDispose {
            barcodeScanner.close()
            analysisExecutor.shutdown()
        }
    }

    AndroidView(
        modifier = modifier,
        factory = { context ->
            PreviewView(context).apply {
                scaleType = PreviewView.ScaleType.FILL_CENTER
                implementationMode = PreviewView.ImplementationMode.PERFORMANCE
                clipToOutline = true

                // Greyscale via a saturation-0 colour matrix render-effect.
                // Hardware-accelerated, only available on Android 12 (API 31)
                // and up — older devices fall back to colour preview, which
                // still scans correctly.
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    val matrix = ColorMatrix().apply { setSaturation(0f) }
                    val filter = ColorMatrixColorFilter(matrix)
                    setRenderEffect(RenderEffect.createColorFilterEffect(filter))
                }
            }
        },
        update = { previewView ->
            val cameraProviderFuture = ProcessCameraProvider.getInstance(ctx)
            cameraProviderFuture.addListener({
                val cameraProvider = runCatching { cameraProviderFuture.get() }.getOrNull()
                    ?: return@addListener

                val resolutionSelector = ResolutionSelector.Builder()
                    .setResolutionStrategy(
                        ResolutionStrategy(
                            Size(1280, 720),
                            ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER,
                        ),
                    )
                    .build()

                val preview = Preview.Builder()
                    .setResolutionSelector(resolutionSelector)
                    .build()
                    .also { it.surfaceProvider = previewView.surfaceProvider }

                val analysis = ImageAnalysis.Builder()
                    .setResolutionSelector(resolutionSelector)
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .build()

                analysis.setAnalyzer(analysisExecutor) { proxy ->
                    processFrame(
                        proxy = proxy,
                        scanner = barcodeScanner,
                        delivered = delivered,
                        onScanned = { onScannedState.value(it) },
                    )
                }

                runCatching {
                    cameraProvider.unbindAll()
                    cameraProvider.bindToLifecycle(
                        lifecycleOwner,
                        CameraSelector.DEFAULT_BACK_CAMERA,
                        preview,
                        analysis,
                    )
                }.onFailure { e ->
                    Log.w(TAG, "bindToLifecycle failed", e)
                }
            }, ContextCompat.getMainExecutor(ctx))
        },
    )
}

private fun processFrame(
    proxy: ImageProxy,
    scanner: com.google.mlkit.vision.barcode.BarcodeScanner,
    delivered: java.util.concurrent.atomic.AtomicBoolean,
    onScanned: (String) -> Unit,
) {
    val mediaImage = proxy.image
    if (mediaImage == null || delivered.get()) {
        proxy.close()
        return
    }
    val rotation = proxy.imageInfo.rotationDegrees
    val input = InputImage.fromMediaImage(mediaImage, rotation)

    scanner.process(input)
        .addOnSuccessListener { barcodes ->
            val first = barcodes.firstOrNull { !it.rawValue.isNullOrBlank() }
            if (first != null && delivered.compareAndSet(false, true)) {
                onScanned(first.rawValue ?: first.displayValue.orEmpty())
            }
        }
        .addOnCompleteListener { proxy.close() }
}

@Composable
private fun CameraPermissionFallback(
    onRequest: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier = modifier
            .background(Color(0xFF6B6B6B))
            .clickable { onRequest() },
        contentAlignment = Alignment.Center,
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(
                imageVector = OhdIcons.ScanBarcode,
                contentDescription = null,
                tint = Color.White.copy(alpha = 0.7f),
                modifier = Modifier.size(28.dp),
            )
            Text(
                text = "Tap to enable camera",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 13.sp,
                color = Color.White.copy(alpha = 0.85f),
            )
        }
    }
}
