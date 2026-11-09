package io.github.yashwanthkamireddi.skykeep

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentType
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.Account
import uniffi.quince_mobile.EngineException
import uniffi.quince_mobile.SignIn

private sealed interface Step {
  data object Credentials : Step
  data object Code : Step
}

/**
 * Signing in with the Apple Account: the desktop app's own sign-in, in Android's form. Fields
 * carry autofill hints, so a password manager fills them; the code field takes the six digits
 * from the SMS keyboard suggestion. Every call runs off the main thread.
 */
@Composable
fun SignInScreen(account: Account, onSignedIn: () -> Unit) {
  var step by remember { mutableStateOf<Step>(Step.Credentials) }
  var appleId by remember { mutableStateOf("") }
  var password by remember { mutableStateOf("") }
  var code by remember { mutableStateOf("") }
  var busy by remember { mutableStateOf(false) }
  var error by remember { mutableStateOf<String?>(null) }
  val scope = rememberCoroutineScope()

  fun run(block: () -> Unit) {
    if (busy) return
    busy = true
    error = null
    scope.launch {
      val failure = withContext(Dispatchers.IO) {
        try { block(); null } catch (e: EngineException.Failed) { e.reason }
      }
      busy = false
      error = failure
    }
  }

  Column(
    Modifier.fillMaxWidth().imePadding().padding(horizontal = 32.dp, vertical = 96.dp).widthIn(max = 420.dp),
    verticalArrangement = Arrangement.spacedBy(16.dp),
    horizontalAlignment = Alignment.Start,
  ) {
    Text("Sign in with your Apple Account", style = MaterialTheme.typography.headlineSmall)
    when (step) {
      Step.Credentials -> {
        OutlinedTextField(
          value = appleId, onValueChange = { appleId = it }, label = { Text("Email or phone number") },
          singleLine = true, enabled = !busy,
          keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email, imeAction = ImeAction.Next),
          modifier = Modifier.fillMaxWidth().semantics { contentType = ContentType.Username },
        )
        val submit = {
          run {
            when (account.signIn(appleId.trim(), password)) {
              SignIn.COMPLETE -> scope.launch { onSignedIn() }
              SignIn.NEEDS_CODE -> step = Step.Code
            }
            password = ""
          }
        }
        OutlinedTextField(
          value = password, onValueChange = { password = it }, label = { Text("Password") },
          singleLine = true, enabled = !busy, visualTransformation = PasswordVisualTransformation(),
          keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Go),
          keyboardActions = KeyboardActions(onGo = { submit() }),
          modifier = Modifier.fillMaxWidth().semantics { contentType = ContentType.Password },
        )
        Button(onClick = { submit() }, enabled = !busy && appleId.isNotBlank() && password.isNotEmpty()) {
          Text("Sign In")
        }
      }
      Step.Code -> {
        Text("Enter the six-digit code shown on your other Apple devices.", style = MaterialTheme.typography.bodyMedium)
        val submit = { run { account.submitCode(code); scope.launch { onSignedIn() } } }
        OutlinedTextField(
          value = code, onValueChange = { v -> code = v.filter(Char::isDigit).take(6); if (code.length == 6) submit() },
          label = { Text("Code") }, singleLine = true, enabled = !busy,
          keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword, imeAction = ImeAction.Done),
          modifier = Modifier.fillMaxWidth().semantics { contentType = ContentType.SmsOtpCode },
        )
        TextButton(onClick = { run { account.resendToDevices() } }, enabled = !busy) { Text("Send a new code") }
      }
    }
    if (busy) CircularProgressIndicator(Modifier.padding(top = 8.dp))
    error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium) }
  }
}
