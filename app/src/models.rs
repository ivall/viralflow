//! Slint models that are updated in place. Replacing a model makes Slint
//! rebuild every element of its repeater, which drops a pointer grab in the
//! middle of a drag (and costs time on every frame); changing rows keeps them.

use std::rc::Rc;

use slint::{Model, VecModel};

/// Makes `model` hold `rows`, touching only the rows that differ.
pub fn sync_rows<T: Clone + PartialEq + 'static>(model: &Rc<VecModel<T>>, rows: Vec<T>) {
    let count = rows.len();
    for (index, row) in rows.into_iter().enumerate() {
        if index < model.row_count() {
            if model.row_data(index).as_ref() != Some(&row) {
                model.set_row_data(index, row);
            }
        } else {
            model.push(row);
        }
    }
    while model.row_count() > count {
        model.remove(model.row_count() - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_equal_rows_and_resizes() {
        let model = Rc::new(VecModel::from(vec![1, 2, 3]));
        sync_rows(&model, vec![1, 5]);
        assert_eq!(model.iter().collect::<Vec<_>>(), [1, 5]);
        sync_rows(&model, vec![1, 5, 7, 9]);
        assert_eq!(model.iter().collect::<Vec<_>>(), [1, 5, 7, 9]);
        sync_rows(&model, vec![]);
        assert_eq!(model.row_count(), 0);
    }
}
