extern crate proc_macro;

use quote::quote;
use rand::{distributions::Uniform, Rng};
use syn::{parse_macro_input, ItemFn};

#[proc_macro_attribute]
pub fn postgres(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let db_name = format!(
        "test_{}",
        rand::thread_rng()
            .sample_iter(Uniform::new(char::from(97), char::from(122)))
            .take(8)
            .collect::<String>()
    );
    let fixtures: Vec<String> = args
        .into_iter()
        .map(|x| x.to_string().replace("\"", ""))
        .collect();

    // Parse input function and function name
    let input_fn = parse_macro_input!(input as ItemFn);
    let test_fn_ident = &input_fn.sig.ident;

    let base_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let setup_code = quote! {
        let mut pg_config = tokio_postgres::Config::new()
            .user(&"postgres")
            .password(&"password")
            .host(&"localhost")
            .port(5432).clone();

        let mgr = deadpool_postgres::Manager::from_config(
            pg_config.clone(),
            tokio_postgres::NoTls,
            deadpool_postgres::ManagerConfig::default(),
        );
        let pool = deadpool_postgres::Pool::builder(mgr)
            .max_size(1)
            .build()
            .unwrap();
        let client = pool.get().await.unwrap();

        // Create a database
        client.execute(&*format!("CREATE DATABASE {};", &#db_name), &[]).await.unwrap();

        // Connect to the new database
        pg_config.dbname(&#db_name.clone());
        let mgr = deadpool_postgres::Manager::from_config(pg_config.clone(), tokio_postgres::NoTls, deadpool_postgres::ManagerConfig::default());
        let pool = deadpool_postgres::Pool::builder(mgr)
        .max_size(1)
        .build().unwrap();

        // Run migrations
        let base_dir = PathBuf::from(#base_dir);
        let up_migrations = std::fs::read_to_string(base_dir.join(PathBuf::from("migrations/20221223050143_base_0.up.sql"))).unwrap();
        client.batch_execute(&up_migrations).await.unwrap();

        // Run test fixturepg_configs
        for fixture in &[#(#fixtures),*] {
            let fixture_sql = std::fs::read_to_string(base_dir.join(PathBuf::from(&format!("fixtures/{}", fixture)))).unwrap();
            client.batch_execute(&fixture_sql).await.unwrap();
        }
    };

    let teardown_code = quote! {
        let mut pg_config = tokio_postgres::Config::new()
            .user(&"postgres")
            .password(&"password")
            .host(&"localhost")
            .port(5432).clone();

        let mgr = deadpool_postgres::Manager::from_config(
            pg_config.clone(),
            tokio_postgres::NoTls,
            deadpool_postgres::ManagerConfig::default(),
        );
        let pool = deadpool_postgres::Pool::builder(mgr)
            .max_size(1)
            .build()
            .unwrap();

        let client = pool.get().await.unwrap();
            client
                .execute(
                    "SELECT pid, pg_terminate_backend(pid)
                        FROM pg_stat_activity
                        WHERE datname = $1 AND pid <> pg_backend_pid()",
                    &[&#db_name],
                )
                .await
                .unwrap();
            client
                .execute("DROP SCHEMA PUBLIC CASCADE", &[])
                .await
                .unwrap();
            client.execute("CREATE SCHEMA PUBLIC", &[]).await.unwrap();
    };

    // Wrap the test in the generated code
    let wrapped_test = quote! {
        #[tokio::test]
        async fn #test_fn_ident() {
            let setup = std::panic::catch_unwind(|| async {
                #setup_code
            });
            if let Err(err) = setup {
                println!("Err in proc macro setup {:?}", err);
            }

            let test = std::panic::catch_unwind(|| async {
                #input_fn
            });

            let teardown = std::panic::catch_unwind(|| async {
                #teardown_code
            });

            if let Err(err) = teardown {
                println!("Err in proc macro teardown {:?}", err);
            }
            if let Err(err) = test {
                std::panic::resume_unwind(err);
            }
        }
    };

    wrapped_test.into()
}
