# R8 shrinks the Kotlin shell in release builds. Everything the Rust library
# reaches by *name* over JNI has to survive: the methods it calls on the
# activity, and the native methods it implements.
-keep class io.github.nujufas.jsonquery.MainActivity {
    public void pickOpen(int);
    public void pickSave(int, java.lang.String);
    public void fileSaved(java.lang.String);
    public java.lang.String clipboardText();
    public void setClipboardText(java.lang.String);
    public void setKeyboardVisible(boolean);
    public void setDarkTheme(boolean);
}
-keepclasseswithmembernames class io.github.nujufas.jsonquery.Native {
    native <methods>;
}
