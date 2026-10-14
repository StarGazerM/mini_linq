//! Callback capture must retain the originating Rust macro's local hygiene.
mod consumer {
    macro_rules! leaf {
        (struct $q:ident; relation $r:ident($c:ident: $t:ty) = ($e:expr);
         $head:ident($v:ident) :- $atom:ident($a:ident);) => {
            $e
        };
    }
    pub(crate) use leaf;
}
macro_rules! fixture {
    ($local:ident) => {
        #[test]
        // Intentionally export from a local scope to exercise captured locals.
        #[allow(non_local_definitions)]
        fn callback_preserves_local_hygiene_and_accepts_a_path() {
            let $local = [(41,)];
            ::mini_linq::workload_query! {
                struct CallbackHygiene;
                relation R(a: i32) = (&$local);
                callback_answer(a) :- R(a);
            }
            // A string/parse round trip loses the outer macro's local context.
            assert_eq!(with_query!(consumer::leaf), &[(41,)]);
            assert!(RUST_PATH.ends_with("workload_callback.rs"));
            assert_eq!(module().unwrap().name, "CallbackHygiene");
        }
    };
}
fixture!(captured);
