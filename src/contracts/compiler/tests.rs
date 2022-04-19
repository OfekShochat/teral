#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::contracts::compiler::{lex, Compiler};

    use super::*;

    #[test]
    fn if_else() {
        let input = lex(r#"
fn transfer from to amount in
    amount 100_u8 >
    require
    0_u8
    if
        10
    else
        11
    end
    100 get
end"#
            .to_string());
        let mut compiler = Compiler::new(input);
        if let Err(err) = compiler.advance() {
            assert!(false, "{}", err);
        }
        let mut expected_functions = HashMap::new();
        expected_functions.insert(
            "transfer".to_string(),
            (
                0_usize,
                vec!["from".to_string(), "to".to_string(), "amount".to_string()],
            ),
        );
        assert_eq!(expected_functions, compiler.functions.clone());

        let expected_output = vec![
            76, 7, 100, 177, 7, 1, 180, 0, 7, 0, 7, 36, 72, 38, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 33, 73, 38, 11, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            38, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 6,
        ];
        assert_eq!(expected_output, compiler.output.clone());
    }

    #[test]
    fn only_if() {
        let input = lex(r#"
fn transfer from to amount in
    0_u8
    if
        10
    end

    from amount +
    if
        20
    end
end"#
            .to_string());
        let mut compiler = Compiler::new(input);
        if let Err(err) = compiler.advance() {
            assert!(false, "{}", err);
        }
        let mut expected_functions = HashMap::new();
        expected_functions.insert(
            "transfer".to_string(),
            (
                0_usize,
                vec!["from".to_string(), "to".to_string(), "amount".to_string()],
            ),
        );
        assert_eq!(expected_functions, compiler.functions.clone());

        let expected_output = vec![
            7, 0, 7, 33, 72, 38, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 74, 76, 1, 7, 33, 72, 38, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(expected_output, compiler.output.clone());
    }

    #[test]
    fn iszero() {
        let input = lex(r#"
fn transfer from to amount in
    10
    iszero if
        amount +
    end
end"#
            .to_string());
        let mut compiler = Compiler::new(input);
        if let Err(err) = compiler.advance() {
            assert!(false, "{}", err);
        }
        let mut expected_functions = HashMap::new();
        expected_functions.insert(
            "transfer".to_string(),
            (
                0_usize,
                vec!["from".to_string(), "to".to_string(), "amount".to_string()],
            ),
        );
        assert_eq!(expected_functions, compiler.functions.clone());

        let expected_output = vec![
            38, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 181, 7, 2, 72, 76, 1,
        ];
        assert_eq!(expected_output, compiler.output.clone());
    }
}
