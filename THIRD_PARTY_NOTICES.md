# Third-party attribution

The MIT license in this repository covers Cash Ledger's authored code,
documentation and application icon. It does not replace third-party licenses.

- GPUI (`gpui-unofficial`) and its platform packages are community distributions
  of Zed's GPUI framework, licensed Apache-2.0.
- GPUI Component and its asset package (`gpui-component-uo` and
  `gpui-component-assets-uo`) are licensed Apache-2.0. Their embedded icon
  assets retain their upstream notices, including Lucide's ISC license.
  The [Lucide/Feather license](third-party/LUCIDE-LICENSE.txt) is included
  explicitly because the published asset crate omits that license file.
  [Apache-2.0](third-party/APACHE-2.0.txt) is also supplied for crates declaring
  that license without including its text. These texts are embedded in the
  generated release notices. Sources: [Lucide](https://github.com/lucide-icons/lucide/blob/main/LICENSE)
  and [Apache](https://www.apache.org/licenses/LICENSE-2.0.txt).
- `rusqlite` is MIT-licensed; SQLite itself is in the public domain.
- Other Rust dependencies retain the license expressions and notices shipped
  in their crate sources. The packaging script generates
  `THIRD_PARTY_LICENSES.txt` from the locked Windows dependency graph, including
  package authors, license declarations, repository links and available license
  and notice files. Build and test dependencies may also appear in that inventory.
- The app-local `vcruntime140.dll` is Microsoft software, not MIT-licensed.
  Copyright Microsoft Corporation. Redistribution is governed by the licensed
  Visual Studio installation's redistributable terms. See Microsoft's
  [redistribution documentation](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files).
- The installer is produced with Inno Setup. Its license is distributed with
  the compiler; Inno Setup requests that commercial users
  [purchase a license](https://jrsoftware.org/ishelp/topic_purchase.htm).

Windows release installers include the project LICENSE, this attribution file,
and the generated dependency notices in the installation directory.
