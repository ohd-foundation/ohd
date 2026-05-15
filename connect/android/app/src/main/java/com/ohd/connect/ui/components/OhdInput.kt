package com.ohd.connect.ui.components

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Text input — Pencil `SipDH`.
 *
 * Height **44 dp**, padding `[h=12, v=0]`, corner `radius-md`, 1.5 dp
 * `ohd-line` border, fill `ohd-bg`, placeholder `Inter 14 / normal /
 * ohd-muted`.
 *
 * The optional [leadingIcon] sits left of the text field at 18 dp tinted
 * `ohd-muted`. Used for the search-with-magnifier pattern in Food v3.
 */
@Composable
fun OhdInput(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
    modifier: Modifier = Modifier,
    leadingIcon: ImageVector? = null,
    keyboardType: KeyboardType = KeyboardType.Text,
    singleLine: Boolean = true,
) {
    val shape = RoundedCornerShape(8.dp)
    val textStyle = TextStyle(
        fontFamily = OhdBody,
        fontWeight = FontWeight.W400,
        fontSize = 14.sp,
        color = OhdColors.Ink,
    )

    Row(
        modifier = modifier
            .height(44.dp)
            .fillMaxWidth()
            .background(OhdColors.Bg, shape)
            .border(BorderStroke(1.5.dp, OhdColors.Line), shape)
            .padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (leadingIcon != null) {
            Icon(
                imageVector = leadingIcon,
                contentDescription = null,
                tint = OhdColors.Muted,
                modifier = Modifier.size(18.dp),
            )
        }

        BasicTextField(
            value = value,
            onValueChange = onValueChange,
            modifier = Modifier.weight(1f),
            singleLine = singleLine,
            textStyle = textStyle,
            cursorBrush = SolidColor(OhdColors.Ink),
            keyboardOptions = KeyboardOptions(keyboardType = keyboardType),
            decorationBox = { inner ->
                if (value.isEmpty()) {
                    Text(
                        text = placeholder,
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 14.sp,
                        color = OhdColors.Muted,
                    )
                }
                inner()
            },
        )
    }
}

/**
 * Labelled input — Pencil `d19IvB`.
 *
 * Vertical column gap 6: label / input / helper. The input itself uses
 * [OhdInput] so it inherits the height/border/placeholder rules.
 */
@Composable
fun OhdField(
    label: String,
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String = "",
    helper: String? = null,
    leadingIcon: ImageVector? = null,
    keyboardType: KeyboardType = KeyboardType.Text,
) {
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 13.sp,
            color = OhdColors.Ink,
        )
        OhdInput(
            value = value,
            onValueChange = onValueChange,
            placeholder = placeholder,
            leadingIcon = leadingIcon,
            keyboardType = keyboardType,
        )
        if (helper != null) {
            Text(
                text = helper,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.Muted,
            )
        }
    }
}
