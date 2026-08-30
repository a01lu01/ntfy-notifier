package app.ntfy.notifier

/** API 24-safe replacements for JDK classes and reflection methods used by Jackson 2.15.3. */
object JacksonApi24Compat {
  /**
   * Avoids a verifier-time reference to BootstrapMethodError, which Android added in API 26.
   * This must stay an exact name check: Jackson intentionally treats other LinkageErrors, such
   * as the missing optional java.nio.file.Path implementation on API 24, as recoverable.
  */
  @JvmStatic
  fun isBootstrapMethodError(throwable: Throwable?): Boolean {
    var type: Class<*>? = throwable?.javaClass
    while (type != null) {
      if (type.name == "java.lang.BootstrapMethodError") return true
      type = type.superclass
    }
    return false
  }
}
