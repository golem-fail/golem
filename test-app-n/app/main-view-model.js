import { Observable } from '@nativescript/core'

export function createViewModel() {
  const viewModel = new Observable()
  viewModel.count = 0

  viewModel.onIncrement = () => {
    viewModel.set('count', viewModel.count + 1)
  }
  viewModel.onDecrement = () => {
    viewModel.set('count', viewModel.count - 1)
  }

  return viewModel
}
