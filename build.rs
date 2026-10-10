use quote::{format_ident, quote};
use std::{env, fs, path::PathBuf};
use syn::{Expr, Item, Lit};

fn main() {
    const DEFINITIONS: &[&str] = &[
        "src/executor/command_definitions.rs",
        "src/executor/recorder/command_definitions.rs",
    ];
    let mut items = Vec::new();
    for definitions in DEFINITIONS {
        println!("cargo:rerun-if-changed={definitions}");
        let source = fs::read_to_string(definitions).expect("read interactive command definitions");
        let file = syn::parse_file(&source).expect("parse interactive command definitions");
        items.extend(file.items);
    }
    let mut variants = Vec::new();
    let mut functions = Vec::new();
    let mut specs = Vec::new();
    let mut dispatch_arms = Vec::new();
    let mut callback_arms = Vec::new();
    let mut next_dialog_trigger = 1u8;

    for item in items {
        let Item::Fn(mut function) = item else {
            panic!("interactive command definitions may only contain functions");
        };
        let attribute_index = function
            .attrs
            .iter()
            .position(|attribute| {
                attribute.path().is_ident("main_command")
                    || attribute.path().is_ident("recorder_command")
            })
            .expect("command function is missing #[main_command(...)] or #[recorder_command(...)]");
        let attribute = function.attrs.remove(attribute_index);
        let scope = if attribute.path().is_ident("recorder_command") {
            quote!(CommandScope::Recorder)
        } else {
            quote!(CommandScope::Main)
        };

        let mut id = None;
        let mut aliases = Vec::new();
        let mut help = None;
        let mut label = None;
        attribute
            .parse_nested_meta(|meta| {
                if meta.path.is_ident("id") {
                    id = Some(meta.value()?.parse::<syn::LitStr>()?.value());
                } else if meta.path.is_ident("aliases") {
                    let expression = meta.value()?.parse::<Expr>()?;
                    let Expr::Array(array) = expression else {
                        return Err(meta.error("aliases must be a string array"));
                    };
                    for element in array.elems {
                        let Expr::Lit(literal) = element else {
                            return Err(meta.error("alias must be a string"));
                        };
                        let Lit::Str(value) = literal.lit else {
                            return Err(meta.error("alias must be a string"));
                        };
                        aliases.push(value.value());
                    }
                } else if meta.path.is_ident("help") {
                    help = Some(meta.value()?.parse::<syn::LitStr>()?.value());
                } else if meta.path.is_ident("label") {
                    label = Some(meta.value()?.parse::<syn::LitStr>()?.value());
                } else {
                    return Err(meta.error("unknown command property"));
                }
                Ok(())
            })
            .expect("invalid recorder command attribute");

        let variant = format_ident!("{}", id.expect("command id is required"));
        let function_name = function.sig.ident.clone();
        let help_tokens = help
            .map(|value| quote!(Some(#value)))
            .unwrap_or_else(|| quote!(None));
        let dialog_tokens = match label {
            Some(label) => {
                let trigger = next_dialog_trigger;
                next_dialog_trigger = next_dialog_trigger
                    .checked_add(1)
                    .expect("too many recorder dialog commands");
                quote!(Some(DialogAction { trigger: #trigger, label: #label }))
            }
            None => quote!(None),
        };
        let callback = function_name.to_string();

        variants.push(variant.clone());
        specs.push(quote! {
            FlintCommandSpec {
                command: FlintCommand::#variant,
                scope: #scope,
                aliases: &[#(#aliases),*],
                help: #help_tokens,
                dialog: #dialog_tokens,
                callback: #callback,
            }
        });
        dispatch_arms.push(quote! {
            FlintCommand::#variant => #function_name(executor, context)?
        });
        callback_arms.push(quote! {
            FlintCommand::#variant => #callback
        });
        functions.push(function);
    }

    let generated = quote! {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum FlintCommand { #(#variants),* }

        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum CommandScope {
            Main,
            Recorder,
        }

        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct DialogAction {
            pub trigger: u8,
            pub label: &'static str,
        }

        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct FlintCommandSpec {
            pub command: FlintCommand,
            pub scope: CommandScope,
            pub aliases: &'static [&'static str],
            pub help: Option<&'static str>,
            pub dialog: Option<DialogAction>,
            pub callback: &'static str,
        }

        pub const COMMANDS: &[FlintCommandSpec] = &[#(#specs),*];

        pub fn from_chat(command: &str) -> Option<FlintCommand> {
            COMMANDS.iter().find(|spec| spec.aliases.contains(&command)).map(|spec| spec.command)
        }

        pub fn from_callback(callback: &str) -> Option<FlintCommand> {
            COMMANDS.iter().find(|spec| spec.callback == callback && spec.dialog.is_some()).map(|spec| spec.command)
        }

        pub const fn callback_id(command: FlintCommand) -> &'static str {
            match command { #(#callback_arms),* }
        }

        pub fn primary_alias(command: FlintCommand) -> &'static str {
            COMMANDS.iter().find(|spec| spec.command == command)
                .and_then(|spec| spec.aliases.first().copied())
                .expect("every recorder command must have a primary alias")
        }

        pub fn dispatch(
            executor: &mut TestExecutor,
            command: FlintCommand,
            context: &mut FlintCommandContext<'_>,
        ) -> Result<()> {
            let spec = COMMANDS.iter().find(|spec| spec.command == command)
                .expect("generated command must have a specification");
            if spec.scope == CommandScope::Recorder && executor.recorder.is_none() {
                executor.bot.send_command(
                    "say No recording in progress. Use !record <name> to start."
                )?;
                return Ok(());
            }
            match command { #(#dispatch_arms),* }
            Ok(())
        }

        #(#functions)*
    };

    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("flint_commands.rs");
    fs::write(output, generated.to_string()).expect("write generated recorder commands");
}
