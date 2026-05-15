package com.ohd.connect.data

import android.content.Context
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning

/**
 * Thin wrapper around Google's Code Scanner (Play Services) so screens
 * don't import the GMS classes directly.
 *
 * The scanner runs in the Play-services process — we don't need to declare
 * `android.permission.CAMERA` in our manifest. Google's UI handles the
 * permission prompt + camera preview + barcode detection. On success the
 * caller gets back the raw scanned value (string for QR / EAN / UPC, etc.).
 *
 * If Google Code Scanner isn't available (e.g. AOSP without GMS), the
 * `Task` resolves with an exception; we surface that via [onError].
 */
object BarcodeScanner {

    /**
     * Open the scanner. Calls [onResult] with the barcode's raw string on
     * success, or [onError] with a short user-facing message on failure or
     * cancellation.
     */
    fun launch(
        ctx: Context,
        onResult: (String) -> Unit,
        onError: (String) -> Unit = {},
    ) {
        // Restrict to the formats food labels actually use. Including QR
        // makes barcode-on-QR-of-product-page (e.g. some store apps) work
        // without extra config.
        val options = GmsBarcodeScannerOptions.Builder()
            .setBarcodeFormats(
                Barcode.FORMAT_EAN_13,
                Barcode.FORMAT_EAN_8,
                Barcode.FORMAT_UPC_A,
                Barcode.FORMAT_UPC_E,
                Barcode.FORMAT_CODE_128,
                Barcode.FORMAT_QR_CODE,
            )
            .build()

        val scanner = GmsBarcodeScanning.getClient(ctx, options)
        scanner.startScan()
            .addOnSuccessListener { barcode ->
                val value = barcode.rawValue ?: barcode.displayValue
                if (value.isNullOrBlank()) {
                    onError("Couldn't read the barcode")
                } else {
                    onResult(value)
                }
            }
            .addOnCanceledListener { onError("Scan cancelled") }
            .addOnFailureListener { e ->
                onError("Scanner unavailable: ${e.message ?: "(unknown)"}")
            }
    }
}
