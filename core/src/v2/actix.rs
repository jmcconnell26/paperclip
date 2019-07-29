use super::models::{DefaultSchemaRaw, Operation, Parameter, ParameterIn, Response};
use super::schema::{Apiv2Operation, Apiv2Schema};
use actix_web::web::{Form, Json, Path, Query};
use futures::future::Future;

use std::collections::BTreeMap;

/// Actix-specific trait for indicating that this entity can modify an operation
/// and/or update the global map of definitions.
pub trait OperationModifier: Apiv2Schema {
    /// Update the parameters list in the given operation (if needed).
    fn update_parameter(_op: &mut Operation<DefaultSchemaRaw>) {}

    /// Update the responses map in the given operation (if needed).
    fn update_response(_op: &mut Operation<DefaultSchemaRaw>) {}

    /// Update the definitions map (if needed).
    fn update_definitions(map: &mut BTreeMap<String, DefaultSchemaRaw>) {
        let mut schema = Self::schema_with_ref();
        loop {
            if let Some(s) = schema.items {
                schema = *s;
                continue;
            } else if let Some(s) = schema.extra_props {
                schema = *s;
                continue;
            } else if let Some(n) = schema.name.take() {
                schema.remove_refs();
                map.insert(n, schema);
            }

            break;
        }
    }
}

/// All schema types default to updating the definitions map.
impl<T> OperationModifier for T where T: Apiv2Schema {}

impl<T> OperationModifier for Option<T>
where
    T: OperationModifier,
{
    fn update_parameter(op: &mut Operation<DefaultSchemaRaw>) {
        T::update_parameter(op);
    }

    fn update_response(op: &mut Operation<DefaultSchemaRaw>) {
        T::update_response(op);
    }

    fn update_definitions(map: &mut BTreeMap<String, DefaultSchemaRaw>) {
        T::update_definitions(map);
    }
}

impl<T, E> OperationModifier for Result<T, E>
where
    T: OperationModifier,
{
    fn update_parameter(op: &mut Operation<DefaultSchemaRaw>) {
        T::update_parameter(op);
    }

    default fn update_response(op: &mut Operation<DefaultSchemaRaw>) {
        T::update_response(op);
    }

    default fn update_definitions(map: &mut BTreeMap<String, DefaultSchemaRaw>) {
        T::update_definitions(map);
    }
}

impl<T, E> Apiv2Schema for dyn Future<Item = T, Error = E> {}

impl<T: Apiv2Schema, E> Apiv2Schema for dyn Future<Item = T, Error = E> {
    const NAME: Option<&'static str> = T::NAME;

    fn raw_schema() -> DefaultSchemaRaw {
        T::raw_schema()
    }
}

impl<T, E> OperationModifier for dyn Future<Item = T, Error = E>
where
    T: OperationModifier,
{
    fn update_parameter(op: &mut Operation<DefaultSchemaRaw>) {
        T::update_parameter(op);
    }

    default fn update_response(op: &mut Operation<DefaultSchemaRaw>) {
        T::update_response(op);
    }

    default fn update_definitions(map: &mut BTreeMap<String, DefaultSchemaRaw>) {
        T::update_definitions(map);
    }
}

impl<T> Apiv2Schema for Json<T> {}

/// JSON needs specialization because it updates the global definitions.
impl<T: Apiv2Schema> Apiv2Schema for Json<T> {
    const NAME: Option<&'static str> = T::NAME;

    fn raw_schema() -> DefaultSchemaRaw {
        T::raw_schema()
    }
}

impl<T> OperationModifier for Json<T>
where
    T: Apiv2Schema,
{
    fn update_parameter(op: &mut Operation<DefaultSchemaRaw>) {
        op.parameters.push(Parameter {
            description: None,
            in_: ParameterIn::Body,
            name: "body".into(),
            required: true,
            schema: Some({
                let mut def = T::schema_with_ref();
                def.retain_ref();
                def
            }),
            data_type: None,
            format: None,
            items: None,
            enum_: Default::default(),
        });
    }

    fn update_response(op: &mut Operation<DefaultSchemaRaw>) {
        op.responses.insert(
            "200".into(),
            Response {
                description: None,
                schema: Some({
                    let mut def = T::schema_with_ref();
                    def.retain_ref();
                    def
                }),
            },
        );
    }
}

macro_rules! impl_param_extractor ({ $ty:ty => $container:ident } => {
    impl<T> Apiv2Schema for $ty {}

    impl<T: Apiv2Schema> OperationModifier for $ty {
        fn update_parameter(op: &mut Operation<DefaultSchemaRaw>) {
            let def = T::raw_schema();
            for (k, v) in def.properties {
                op.parameters.push(Parameter {
                    description: None,
                    in_: ParameterIn::$container,
                    required: def.required.contains(&k),
                    name: k,
                    schema: None,
                    data_type: v.data_type,
                    format: v.format,
                    items: None,
                    enum_: v.enum_,
                });
            }
        }
    }
});

/// `formData` can refer to the global definitions.
impl<T: Apiv2Schema> Apiv2Schema for Form<T> {
    const NAME: Option<&'static str> = T::NAME;

    fn raw_schema() -> DefaultSchemaRaw {
        T::raw_schema()
    }
}

impl_param_extractor!(Path<T> => Path);
impl_param_extractor!(Query<T> => Query);
impl_param_extractor!(Form<T> => FormData);

macro_rules! impl_path_tuple ({ $($ty:ident),+ } => {
    impl<$($ty,)+> Apiv2Schema for Path<($($ty,)+)> {}

    impl<$($ty,)+> OperationModifier for Path<($($ty,)+)>
        where $($ty: Apiv2Schema,)+
    {
        fn update_parameter(op: &mut Operation<DefaultSchemaRaw>) {
            // NOTE: We're setting empty name, because we don't know
            // the name in this context. We'll get it when we add services.
            $(
                let def = $ty::raw_schema();
                op.parameters.push(Parameter {
                    name: String::new(),
                    description: None,
                    in_: ParameterIn::Path,
                    required: true,
                    schema: None,
                    data_type: def.data_type,
                    format: def.format,
                    items: None,
                    enum_: def.enum_,
                });
            )+
        }
    }
});

impl_path_tuple!(A);
impl_path_tuple!(A, B);
impl_path_tuple!(A, B, C);
impl_path_tuple!(A, B, C, D);
impl_path_tuple!(A, B, C, D, E);

macro_rules! impl_fn_operation ({ $($ty:ident),* } => {
    impl<U, $($ty,)* R> Apiv2Operation<($($ty,)*), R> for U
        where U: Fn($($ty,)*) -> R,
              $($ty: OperationModifier,)*
              R: OperationModifier,
    {
        fn operation() -> Operation<DefaultSchemaRaw> {
            let mut op = Operation::default();
            $(
                $ty::update_parameter(&mut op);
            )*
            R::update_response(&mut op);
            op
        }

        fn definitions() -> BTreeMap<String, DefaultSchemaRaw> {
            let mut map = BTreeMap::new();
            $(
                $ty::update_definitions(&mut map);
            )*
            R::update_definitions(&mut map);
            map
        }
    }
});

impl_fn_operation!();
impl_fn_operation!(A);
impl_fn_operation!(A, B);
impl_fn_operation!(A, B, C);
impl_fn_operation!(A, B, C, D);
impl_fn_operation!(A, B, C, D, E);
impl_fn_operation!(A, B, C, D, E, F);
impl_fn_operation!(A, B, C, D, E, F, G);
impl_fn_operation!(A, B, C, D, E, F, G, H);
impl_fn_operation!(A, B, C, D, E, F, G, H, I);
impl_fn_operation!(A, B, C, D, E, F, G, H, I, J);
