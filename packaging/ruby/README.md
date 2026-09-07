# Ruby status

Ruby is not a supported release target.

UniFFI 0.29 officially ships Kotlin, Swift, and Python backends. It does not
ship a Ruby backend, and `uniffi-bindgen-ruby` is not available on crates.io.
The similarly named community repository is currently a scaffold rather than a
production generator. Previous release automation silently ignored this and
could report success while producing no Ruby bindings.

Ruby support can be added later through a maintained generator or a dedicated
C-ABI/Fiddle wrapper, but releases must not advertise or publish an unverified
gem until that implementation exists.
