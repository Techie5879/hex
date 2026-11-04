package anomaly.hex.android.testapp

import android.graphics.Color
import android.os.Bundle
import android.text.InputType
import android.view.ViewGroup
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ComposeView

/** Every kind of native editor Hex must handle, and must refuse. */
class FieldsActivity : ComponentActivity() {
    private lateinit var column: LinearLayout

    val multi by lazy { field("Multiline", InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE, "multi") }
    val single by lazy { field("Single line", InputType.TYPE_CLASS_TEXT, "single") }
    val password by lazy { field("Password", InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD, "password") }
    val visiblePassword by lazy { field("Visible password", InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD, "visible_password") }
    val pin by lazy { field("PIN", InputType.TYPE_CLASS_NUMBER or InputType.TYPE_NUMBER_VARIATION_PASSWORD, "pin") }
    val number by lazy { field("Number", InputType.TYPE_CLASS_NUMBER, "number") }
    val cardNumber by lazy { field("Card number", InputType.TYPE_CLASS_TEXT, "card_number") }
    val incognito by lazy {
        field("Incognito-style", InputType.TYPE_CLASS_TEXT, "incognito").apply {
            imeOptions = android.view.inputmethod.EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
        }
    }
    var composeText by mutableStateOf("")

    private fun field(hint: String, type: Int, tag: String) = EditText(this).apply {
        this.hint = hint
        inputType = type
        this.tag = tag
        setTextColor(Color.BLACK)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(32, 120, 32, 32)
        }
        for (view in listOf(multi, single, password, visiblePassword, pin, number, cardNumber)) column.addView(view)
        column.addView(incognito)
        column.addView(ComposeView(this).apply {
            setContent {
                OutlinedTextField(composeText, { composeText = it }, Modifier.fillMaxWidth(), label = { Text("Compose field") })
            }
        })
        setContentView(ScrollView(this).apply { addView(column, ViewGroup.LayoutParams(-1, -2)) })
    }
}
